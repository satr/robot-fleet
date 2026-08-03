use std::{path::Path, sync::Arc, time::Duration};

use anyhow::anyhow;
use chrono::Utc;
use rand::Rng;
use robot_fleet_common::{
    mqtt::parse_mqtt_url,
    types::{
        CommandResultMessage, RobotCommandMessage, RobotSensorEventMessage, StateMessage,
        TelemetryMessage,
    },
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::json;
use tokio::{
    sync::Mutex,
    time::{interval, sleep, MissedTickBehavior},
};
use tracing::{info, warn};

use crate::{
    config::Config,
    metrics::Metrics,
    persistence::{persist_processed_commands, ProcessedCommandRecord, ProcessedCommandStatus},
    state::{AppliedCommand, RobotState},
};

const STARTUP_RECOVERY_REASON: &str = "simulator_restarted_before_completion";

pub(crate) async fn run_robot(
    config: Config,
    state: Arc<Mutex<RobotState>>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    loop {
        let (client, eventloop) = connect_mqtt(&config)?;
        let eventloop_client = client.clone();
        let eventloop_config = config.clone();
        let eventloop_state = state.clone();
        let eventloop_metrics = metrics.clone();
        let mut eventloop_task = tokio::spawn(async move {
            run_eventloop(
                eventloop_config,
                eventloop_state,
                eventloop_metrics,
                eventloop_client,
                eventloop,
            )
            .await
        });

        recover_unfinished_commands(&client, &config, &state).await?;

        for topic in command_subscription_topics(&config.robot_id) {
            if let Err(err) = client.subscribe(topic, QoS::AtLeastOnce).await {
                eventloop_task.abort();
                let _ = eventloop_task.await;
                return Err(err.into());
            }
        }
        publish_state(&client, &config, &state).await?;

        let telemetry_client = client.clone();
        let telemetry_config = config.clone();
        let telemetry_state = state.clone();
        let telemetry_metrics = metrics.clone();
        let mut telemetry_task = tokio::spawn(async move {
            run_telemetry_publisher(
                telemetry_client,
                telemetry_config,
                telemetry_state,
                telemetry_metrics,
            )
            .await
        });

        let state_client = client.clone();
        let state_config = config.clone();
        let state_state = state.clone();
        let mut state_task = tokio::spawn(async move {
            run_state_publisher(state_client, state_config, state_state).await
        });

        tokio::select! {
            eventloop_result = &mut eventloop_task => {
                match eventloop_result {
                    Ok(Ok(())) => warn!(robot_id = config.robot_id, "MQTT event loop ended; reconnecting"),
                    Ok(Err(err)) => warn!(robot_id = config.robot_id, error = %err, "MQTT disconnected; reconnecting"),
                    Err(err) => warn!(robot_id = config.robot_id, error = %err, "MQTT task failed; reconnecting"),
                }
            }
            telemetry_result = &mut telemetry_task => {
                match telemetry_result {
                    Ok(Ok(())) => warn!(robot_id = config.robot_id, "telemetry publisher ended; reconnecting"),
                    Ok(Err(err)) => warn!(robot_id = config.robot_id, error = %err, "telemetry publisher failed; reconnecting"),
                    Err(err) => warn!(robot_id = config.robot_id, error = %err, "telemetry task failed; reconnecting"),
                }
            }
            state_result = &mut state_task => {
                match state_result {
                    Ok(Ok(())) => warn!(robot_id = config.robot_id, "state publisher ended; reconnecting"),
                    Ok(Err(err)) => warn!(robot_id = config.robot_id, error = %err, "state publisher failed; reconnecting"),
                    Err(err) => warn!(robot_id = config.robot_id, error = %err, "state task failed; reconnecting"),
                }
            }
        }

        eventloop_task.abort();
        telemetry_task.abort();
        state_task.abort();
        let _ = eventloop_task.await;
        let _ = telemetry_task.await;
        let _ = state_task.await;
        sleep(Duration::from_secs(2)).await;
    }
}

fn connect_mqtt(config: &Config) -> anyhow::Result<(AsyncClient, rumqttc::EventLoop)> {
    let (host, port) = parse_mqtt_url(&config.mqtt_url)?;
    let mut options = MqttOptions::new(&config.mqtt_client_id, host, port);
    options.set_keep_alive(Duration::from_secs(10));
    Ok(AsyncClient::new(options, 100))
}

async fn run_telemetry_publisher(
    client: AsyncClient,
    config: Config,
    state: Arc<Mutex<RobotState>>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    let mut telemetry_interval = interval(config.telemetry_interval);
    telemetry_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        telemetry_interval.tick().await;
        publish_telemetry(&client, &config, &state, &metrics).await?;
    }
}

async fn run_state_publisher(
    client: AsyncClient,
    config: Config,
    state: Arc<Mutex<RobotState>>,
) -> anyhow::Result<()> {
    let mut state_interval = interval(config.robot_state_interval);
    state_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        state_interval.tick().await;
        publish_state(&client, &config, &state).await?;
    }
}

async fn run_eventloop(
    config: Config,
    state: Arc<Mutex<RobotState>>,
    metrics: Arc<Metrics>,
    client: AsyncClient,
    mut eventloop: rumqttc::EventLoop,
) -> anyhow::Result<()> {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                metrics.mqtt_connection_status.set(1.0);
                let command_client = client.clone();
                let command_config = config.clone();
                let command_state = state.clone();
                let command_metrics = metrics.clone();
                let payload = publish.payload.to_vec();
                if is_simulated_event_topic(&publish.topic, &config.robot_id) {
                    tokio::spawn(async move {
                        if let Err(err) = handle_simulated_event_request(
                            &command_client,
                            &command_config,
                            &command_state,
                            &command_metrics,
                            &payload,
                        )
                        .await
                        {
                            warn!(
                                robot_id = command_config.robot_id,
                                error = %err,
                                "simulated event handling failed"
                            );
                        }
                    });
                } else {
                    tokio::spawn(async move {
                        if let Err(err) = handle_command(
                            &command_client,
                            &command_config,
                            &command_state,
                            &command_metrics,
                            &payload,
                        )
                        .await
                        {
                            warn!(
                                robot_id = command_config.robot_id,
                                error = %err,
                                "command handling failed"
                            );
                        }
                    });
                }
            }
            Ok(_) => metrics.mqtt_connection_status.set(1.0),
            Err(err) => {
                metrics.mqtt_connection_status.set(0.0);
                return Err(err.into());
            }
        }
    }
}

async fn publish_telemetry(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
    metrics: &Metrics,
) -> anyhow::Result<()> {
    let mut guard = state.lock().await;
    guard.battery_level = (guard.battery_level - 0.2).max(0.0);
    guard.online = rand::thread_rng().gen_bool(0.98);

    let now = Utc::now();
    let telemetry = TelemetryMessage {
        robot_id: config.robot_id.clone(),
        recorded_at: now,
        battery_level: guard.battery_level,
        temperature: rand::thread_rng().gen_range(20.0..=45.0),
        position_x: guard.position_x,
        position_y: guard.position_y,
        velocity_cm_s: guard.velocity,
        direction_degrees: guard.direction_degrees,
        payload: json!({ "online": guard.online }),
    };
    drop(guard);

    client
        .publish(
            format!("robots/{}/telemetry", config.robot_id),
            QoS::AtMostOnce,
            false,
            serde_json::to_vec(&telemetry)?,
        )
        .await?;
    metrics.telemetry_sent.inc();
    Ok(())
}

async fn publish_state(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
) -> anyhow::Result<()> {
    let message = {
        let guard = state.lock().await;
        StateMessage {
            robot_id: config.robot_id.clone(),
            name: config.robot_name.clone(),
            status: if guard.online { "online" } else { "offline" }.into(),
            battery_level: guard.battery_level,
            position_x: guard.position_x,
            position_y: guard.position_y,
            set_velocity: guard.set_velocity,
            velocity: guard.velocity,
            direction_degrees: guard.direction_degrees,
            stop: guard.stop,
            target_position_x: guard.target_position_x,
            target_position_y: guard.target_position_y,
            current_mission: guard.current_mission.clone(),
            state: guard.state.clone(),
            software_version: guard.software_version.clone(),
            recorded_at: Utc::now(),
        }
    };
    client
        .publish(
            format!("robots/{}/state", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message)?,
        )
        .await?;
    Ok(())
}

async fn handle_command(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
    metrics: &Metrics,
    payload: &[u8],
) -> anyhow::Result<()> {
    let command: RobotCommandMessage = serde_json::from_slice(payload)?;
    let command_id = command.command_id;
    if command.robot_id != config.robot_id {
        warn!(
            robot_id = config.robot_id,
            command_robot_id = command.robot_id,
            command_id = %command.command_id,
            "ignoring command for a different robot"
        );
        return Ok(());
    }

    let existing_record = {
        let guard = state.lock().await;
        guard.processed_command(&command.command_id).cloned()
    };
    let is_processing = {
        let guard = state.lock().await;
        guard.command_is_processing(command_id)
    };
    if let Some(existing_record) = &existing_record {
        if !existing_record.matches_command(&command) {
            let cancelled_record = existing_record.with_command_and_status(
                &command,
                ProcessedCommandStatus::Cancelled,
                Utc::now(),
            );
            persist_processed_command_journal(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                |guard| {
                    guard.remember_processed_command(cancelled_record.clone());
                    Ok(cancelled_record.clone())
                },
            )
            .await?;
            info!(
                robot_id = config.robot_id,
                command_id = %command.command_id,
                "command arguments changed; marking command cancelled"
            );
            publish_command_result_with_payload(
                client,
                config,
                command.command_id,
                "failed",
                "command_cancelled",
                json!({
                    "command_type": command.command_type,
                    "payload": command.payload,
                    "reason": "command_arguments_changed",
                }),
            )
            .await?;
            return Ok(());
        }

        let is_active_move = existing_record.status == ProcessedCommandStatus::Running
            && normalize_command_type(&existing_record.command_type) == "move"
            && {
                let guard = state.lock().await;
                guard.move_command_is_active(command_id)
            };
        let can_recover =
            !existing_record.status.is_terminal() && !is_processing && !is_active_move;
        if !can_recover {
            let persisted_status = existing_record.status.duplicate_response();
            if persisted_status != existing_record.status {
                let updated_record = existing_record.with_status(persisted_status, Utc::now());
                persist_processed_command_journal(
                    state,
                    &config.journal_lock,
                    &config.processed_commands_path,
                    |guard| {
                        guard.remember_processed_command(updated_record.clone());
                        Ok(updated_record.clone())
                    },
                )
                .await?;
            }

            info!(
                robot_id = config.robot_id,
                command_id = %command.command_id,
                status = persisted_status.as_str(),
                "duplicate command acknowledged without re-execution"
            );
            publish_command_result(
                client,
                config,
                &command,
                duplicate_publish_status(persisted_status),
                duplicate_command_event_type(persisted_status),
            )
            .await?;
            return Ok(());
        }
    }

    let is_new_command = existing_record.is_none();
    if is_new_command {
        let received_record =
            ProcessedCommandRecord::new(&command, ProcessedCommandStatus::Received, Utc::now());
        persist_processed_command_journal(
            state,
            &config.journal_lock,
            &config.processed_commands_path,
            |guard| {
                guard.remember_processed_command(received_record.clone());
                guard.start_processing_command(command_id);
                Ok(received_record.clone())
            },
        )
        .await?;
        metrics.commands_processed.inc();
    }

    publish_command_result(
        client,
        config,
        &command,
        "acknowledged",
        "command_acknowledged",
    )
    .await?;

    update_command_record_status(
        state,
        &config.journal_lock,
        &config.processed_commands_path,
        command_id,
        ProcessedCommandStatus::Running,
        Utc::now(),
    )
    .await?;

    if command
        .expires_at
        .is_some_and(|expires_at| expires_at <= Utc::now())
    {
        update_command_record_status(
            state,
            &config.journal_lock,
            &config.processed_commands_path,
            command_id,
            ProcessedCommandStatus::Expired,
            Utc::now(),
        )
        .await?;
        publish_command_result(client, config, &command, "expired", "command_expired").await?;
        {
            let mut guard = state.lock().await;
            guard.finish_processing_command(command_id);
        }
        return Ok(());
    }

    let applied = {
        let mut guard = state.lock().await;
        guard.apply_command(command.command_id, &command.command_type, &command.payload)
    };

    let applied = match applied {
        Ok(applied) => applied,
        Err(err) => {
            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command_id,
                ProcessedCommandStatus::Failed,
                Utc::now(),
            )
            .await?;
            {
                let mut guard = state.lock().await;
                guard.finish_processing_command(command_id);
            }
            publish_command_result_with_payload(
                client,
                config,
                command.command_id,
                "failed",
                "command_failed",
                json!({
                    "command_type": command.command_type,
                    "payload": command.payload,
                    "error": err.to_string(),
                }),
            )
            .await?;
            return Ok(());
        }
    };

    match applied {
        AppliedCommand::Move {
            overridden_command_id,
        } => {
            if let Some(overridden_command_id) = overridden_command_id {
                update_command_status_by_id(
                    state,
                    &config.journal_lock,
                    &config.processed_commands_path,
                    overridden_command_id,
                    ProcessedCommandStatus::Stopped,
                    Utc::now(),
                )
                .await?;
                publish_command_result_with_payload(
                    client,
                    config,
                    overridden_command_id,
                    "stopped",
                    "command_stopped",
                    json!({
                        "command_type": "move",
                        "reason": "overridden",
                        "overridden_by": command.command_id,
                    }),
                )
                .await?;
            }

            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command.command_id,
                ProcessedCommandStatus::Running,
                Utc::now(),
            )
            .await?;
            publish_command_result(client, config, &command, "running", "command_running").await?;
            publish_state(client, config, state).await?;
            spawn_move_completion_watcher(client.clone(), config.clone(), state.clone(), command);
            {
                let mut guard = state.lock().await;
                guard.finish_processing_command(command_id);
            }
        }
        AppliedCommand::SetVelocity => {
            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command.command_id,
                ProcessedCommandStatus::Running,
                Utc::now(),
            )
            .await?;
            publish_command_result(client, config, &command, "running", "command_running").await?;
            publish_state(client, config, state).await?;
            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command.command_id,
                ProcessedCommandStatus::Completed,
                Utc::now(),
            )
            .await?;
            publish_command_result(client, config, &command, "completed", "command_completed")
                .await?;
            {
                let mut guard = state.lock().await;
                guard.finish_processing_command(command_id);
            }
        }
        AppliedCommand::Stop {
            stop,
            affected_move_command_id,
        } => {
            if let Some(affected_move_command_id) = affected_move_command_id {
                update_command_status_by_id(
                    state,
                    &config.journal_lock,
                    &config.processed_commands_path,
                    affected_move_command_id,
                    if stop {
                        ProcessedCommandStatus::Stopped
                    } else {
                        ProcessedCommandStatus::Resumed
                    },
                    Utc::now(),
                )
                .await?;
                let (status, event_type) = if stop {
                    ("stopped", "command_stopped")
                } else {
                    ("running", "command_resumed")
                };
                publish_command_result_with_payload(
                    client,
                    config,
                    affected_move_command_id,
                    status,
                    event_type,
                    json!({
                        "command_type": "move",
                        "reason": if stop { "stop_command" } else { "resume_command" },
                        "stop_command_id": command.command_id,
                    }),
                )
                .await?;
            }

            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command.command_id,
                ProcessedCommandStatus::Running,
                Utc::now(),
            )
            .await?;
            publish_command_result(client, config, &command, "running", "command_running").await?;
            publish_state(client, config, state).await?;
            update_command_record_status(
                state,
                &config.journal_lock,
                &config.processed_commands_path,
                command.command_id,
                ProcessedCommandStatus::Completed,
                Utc::now(),
            )
            .await?;
            publish_command_result(client, config, &command, "completed", "command_completed")
                .await?;
            {
                let mut guard = state.lock().await;
                guard.finish_processing_command(command.command_id);
            }
        }
        AppliedCommand::SimulateEvent { .. } => {
            {
                let mut guard = state.lock().await;
                guard.finish_processing_command(command.command_id);
            }
            return Err(anyhow!(
                "simulated event commands are handled separately from command lifecycle"
            ));
        }
    }
    Ok(())
}

async fn handle_simulated_event_request(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
    metrics: &Metrics,
    payload: &[u8],
) -> anyhow::Result<()> {
    let command: RobotCommandMessage = serde_json::from_slice(payload)?;
    let applied = {
        let mut guard = state.lock().await;
        match normalize_command_type(&command.command_type).as_str() {
            "extreme_temperature" => guard.start_simulated_event("extreme_temperature", "high"),
            "robot_stack" => guard.start_simulated_event("robot_stack", "normal"),
            other => return Err(anyhow!("unsupported simulated event command_type: {other}")),
        }
    };

    let AppliedCommand::SimulateEvent {
        event_type,
        priority,
        interrupted_move_command_id,
    } = applied
    else {
        return Err(anyhow!(
            "simulated event command_type produced unexpected command application result"
        ));
    };

    if let Some(interrupted_move_command_id) = interrupted_move_command_id {
        update_command_status_by_id(
            state,
            &config.journal_lock,
            &config.processed_commands_path,
            interrupted_move_command_id,
            ProcessedCommandStatus::Stopped,
            Utc::now(),
        )
        .await?;
        publish_command_result_with_payload(
            client,
            config,
            interrupted_move_command_id,
            "stopped",
            "command_stopped",
            json!({
                "command_type": "move",
                "reason": "sensor_event",
                "event_type": &event_type,
                "stopped_by": command.command_id,
            }),
        )
        .await?;
    }

    publish_state(client, config, state).await?;
    {
        let mut guard = state.lock().await;
        guard.finish_safe_state();
    }
    publish_state(client, config, state).await?;
    publish_sensor_event(client, config, metrics, &command, &event_type, &priority).await?;
    Ok(())
}

async fn recover_unfinished_commands(
    client: &AsyncClient,
    config: &Config,
    state: &Arc<Mutex<RobotState>>,
) -> anyhow::Result<()> {
    let unfinished_commands = {
        let guard = state.lock().await;
        guard
            .processed_commands
            .values()
            .filter(|record| record.needs_recovery())
            .cloned()
            .collect::<Vec<_>>()
    };

    if unfinished_commands.is_empty() {
        return Ok(());
    }

    warn!(
        robot_id = config.robot_id,
        unfinished_commands = unfinished_commands.len(),
        "marking unfinished processed commands as failed after simulator restart"
    );

    for record in unfinished_commands {
        publish_command_result_with_payload(
            client,
            config,
            record.command_id,
            "failed",
            "command_failed",
            startup_recovery_payload(&record),
        )
        .await?;

        let failed_record = record.with_status(ProcessedCommandStatus::Failed, Utc::now());
        persist_processed_command_journal(
            state,
            &config.journal_lock,
            &config.processed_commands_path,
            |guard| {
                guard.remember_processed_command(failed_record.clone());
                Ok(failed_record.clone())
            },
        )
        .await?;
    }

    Ok(())
}

async fn publish_sensor_event(
    client: &AsyncClient,
    config: &Config,
    metrics: &Metrics,
    command: &RobotCommandMessage,
    event_type: &str,
    priority: &str,
) -> anyhow::Result<()> {
    let message = sensor_event_message(&config.robot_id, command, event_type, priority);
    let topic = sensor_event_topic(&config.robot_id, priority);
    client
        .publish(
            topic,
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message)?,
        )
        .await?;
    metrics
        .sensor_events_sent
        .with_label_values(&[event_type, priority])
        .inc();
    Ok(())
}

fn sensor_event_message(
    robot_id: &str,
    command: &RobotCommandMessage,
    event_type: &str,
    priority: &str,
) -> RobotSensorEventMessage {
    RobotSensorEventMessage {
        event_id: uuid::Uuid::new_v4(),
        robot_id: robot_id.into(),
        event_type: event_type.into(),
        priority: priority.into(),
        command_id: None,
        payload: json!({
            "source": "robot_simulator",
            "safe_state_command": "get_to_safe_state",
            "simulated_by_command_type": command.command_type,
            "simulation_request_id": command.command_id,
        }),
        occurred_at: Utc::now(),
    }
}

fn command_subscription_topics(robot_id: &str) -> [String; 3] {
    [
        format!("robots/{robot_id}/commands"),
        format!("robots/{robot_id}/simulated-events"),
        format!("robots/{robot_id}/commands/high-priority"),
    ]
}

fn is_simulated_event_topic(topic: &str, robot_id: &str) -> bool {
    topic == format!("robots/{robot_id}/simulated-events")
        || topic == format!("robots/{robot_id}/commands/high-priority")
}

fn sensor_event_topic(robot_id: &str, priority: &str) -> String {
    if priority == "high" {
        format!("robots/{robot_id}/events/high-priority")
    } else {
        format!("robots/{robot_id}/events")
    }
}

fn normalize_command_type(command_type: &str) -> String {
    command_type
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

fn spawn_move_completion_watcher(
    client: AsyncClient,
    config: Config,
    state: Arc<Mutex<RobotState>>,
    command: RobotCommandMessage,
) {
    tokio::spawn(async move {
        let mut completion_interval = interval(config.robot_state_interval);
        completion_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            completion_interval.tick().await;
            let completed = {
                let mut guard = state.lock().await;
                if guard.take_completed_move_command(command.command_id) {
                    true
                } else if !guard.move_command_is_active(command.command_id) {
                    return;
                } else {
                    false
                }
            };

            if completed {
                if let Err(err) = update_command_record_status(
                    &state,
                    &config.journal_lock,
                    &config.processed_commands_path,
                    command.command_id,
                    ProcessedCommandStatus::Completed,
                    Utc::now(),
                )
                .await
                {
                    warn!(
                        robot_id = config.robot_id,
                        command_id = %command.command_id,
                        error = %err,
                        "failed to persist move completion"
                    );
                    return;
                }
                if let Err(err) = publish_command_result(
                    &client,
                    &config,
                    &command,
                    "completed",
                    "command_completed",
                )
                .await
                {
                    warn!(
                        robot_id = config.robot_id,
                        command_id = %command.command_id,
                        error = %err,
                        "failed to publish move completion"
                    );
                }
                return;
            }
        }
    });
}

async fn publish_command_result(
    client: &AsyncClient,
    config: &Config,
    command: &RobotCommandMessage,
    status: &str,
    event_type: &str,
) -> anyhow::Result<()> {
    publish_command_result_with_payload(
        client,
        config,
        command.command_id,
        status,
        event_type,
        json!({
            "command_type": command.command_type,
            "payload": command.payload,
        }),
    )
    .await
}

async fn publish_command_result_with_payload(
    client: &AsyncClient,
    config: &Config,
    command_id: uuid::Uuid,
    status: &str,
    event_type: &str,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    let message = CommandResultMessage {
        event_id: uuid::Uuid::new_v4(),
        command_id,
        robot_id: config.robot_id.clone(),
        status: status.into(),
        event_type: event_type.into(),
        payload,
        occurred_at: Utc::now(),
    };
    client
        .publish(
            format!("robots/{}/command-results", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&message)?,
        )
        .await?;
    Ok(())
}

fn startup_recovery_payload(record: &ProcessedCommandRecord) -> serde_json::Value {
    json!({
        "command_type": record.command_type.clone(),
        "payload": record.payload.clone(),
        "reason": STARTUP_RECOVERY_REASON,
    })
}

async fn update_command_record_status(
    state: &Arc<Mutex<RobotState>>,
    journal_lock: &Arc<Mutex<()>>,
    path: &Path,
    command_id: uuid::Uuid,
    status: ProcessedCommandStatus,
    updated_at: chrono::DateTime<Utc>,
) -> anyhow::Result<ProcessedCommandRecord> {
    persist_processed_command_journal(state, journal_lock, path, |guard| {
        let current = guard
            .processed_command(&command_id)
            .cloned()
            .ok_or_else(|| anyhow!("missing processed command record for {command_id}"))?;
        let updated_record = current.with_status(status, updated_at);
        guard.remember_processed_command(updated_record.clone());
        Ok(updated_record)
    })
    .await
}

async fn update_command_status_by_id(
    state: &Arc<Mutex<RobotState>>,
    journal_lock: &Arc<Mutex<()>>,
    path: &Path,
    command_id: uuid::Uuid,
    status: ProcessedCommandStatus,
    updated_at: chrono::DateTime<Utc>,
) -> anyhow::Result<ProcessedCommandRecord> {
    update_command_record_status(state, journal_lock, path, command_id, status, updated_at).await
}

async fn persist_processed_command_journal<R, F>(
    state: &Arc<Mutex<RobotState>>,
    journal_lock: &Arc<Mutex<()>>,
    path: &Path,
    mutate: F,
) -> anyhow::Result<R>
where
    F: FnOnce(&mut RobotState) -> anyhow::Result<R>,
{
    let _journal_guard = journal_lock.lock().await;
    let snapshot = {
        let guard = state.lock().await;
        guard.snapshot_processed_command_journal()
    };
    let (result, records) = {
        let mut guard = state.lock().await;
        let result = mutate(&mut guard)?;
        let records = guard.processed_commands.clone();
        (result, records)
    };

    if let Err(err) = persist_processed_commands(path, &records).await {
        let mut guard = state.lock().await;
        guard.restore_processed_command_journal(snapshot);
        return Err(err);
    }

    Ok(result)
}

fn duplicate_command_event_type(status: ProcessedCommandStatus) -> &'static str {
    match status {
        ProcessedCommandStatus::Received | ProcessedCommandStatus::Acknowledged => {
            "command_duplicate_acknowledged"
        }
        ProcessedCommandStatus::Running => "command_running",
        ProcessedCommandStatus::Resumed => "command_resumed",
        ProcessedCommandStatus::Cancelled => "command_cancelled",
        ProcessedCommandStatus::Completed => "command_completed",
        ProcessedCommandStatus::Failed => "command_failed",
        ProcessedCommandStatus::Expired => "command_expired",
        ProcessedCommandStatus::Stopped => "command_stopped",
    }
}

fn duplicate_publish_status(status: ProcessedCommandStatus) -> &'static str {
    match status {
        ProcessedCommandStatus::Received | ProcessedCommandStatus::Acknowledged => "acknowledged",
        ProcessedCommandStatus::Running | ProcessedCommandStatus::Resumed => "running",
        ProcessedCommandStatus::Cancelled
        | ProcessedCommandStatus::Failed
        | ProcessedCommandStatus::Expired => "failed",
        ProcessedCommandStatus::Completed => "completed",
        ProcessedCommandStatus::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use chrono::Utc;
    use serde_json::json;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::{
        command_subscription_topics, is_simulated_event_topic, normalize_command_type,
        sensor_event_message, sensor_event_topic, startup_recovery_payload,
        update_command_record_status, STARTUP_RECOVERY_REASON,
    };
    use crate::persistence::{ProcessedCommandRecord, ProcessedCommandStatus};
    use crate::state::RobotState;

    fn sample_record() -> ProcessedCommandRecord {
        ProcessedCommandRecord::new(
            &robot_fleet_common::types::RobotCommandMessage {
                command_id: Uuid::new_v4(),
                robot_id: "robot-01".into(),
                command_type: "move".into(),
                payload: json!({ "target_position_x": 1.0, "target_position_y": 2.0 }),
                expires_at: None,
            },
            ProcessedCommandStatus::Acknowledged,
            Utc::now(),
        )
    }

    #[test]
    fn simulator_accepts_event_requests_on_current_and_legacy_topics() {
        assert_eq!(
            command_subscription_topics("robot-01"),
            [
                "robots/robot-01/commands".to_string(),
                "robots/robot-01/simulated-events".to_string(),
                "robots/robot-01/commands/high-priority".to_string(),
            ]
        );
    }

    #[test]
    fn sensor_events_use_priority_specific_topics() {
        assert_eq!(
            sensor_event_topic("robot-01", "high"),
            "robots/robot-01/events/high-priority"
        );
        assert_eq!(
            sensor_event_topic("robot-01", "normal"),
            "robots/robot-01/events"
        );
    }

    #[test]
    fn simulated_event_topics_are_detected() {
        assert!(is_simulated_event_topic(
            "robots/robot-01/simulated-events",
            "robot-01"
        ));
        assert!(is_simulated_event_topic(
            "robots/robot-01/commands/high-priority",
            "robot-01"
        ));
        assert!(!is_simulated_event_topic(
            "robots/robot-01/commands",
            "robot-01"
        ));
    }

    #[test]
    fn simulated_event_commands_are_normalized() {
        assert_eq!(normalize_command_type("Robot Stack"), "robot_stack");
    }

    #[test]
    fn simulated_event_message_omits_command_id_and_keeps_request_id() {
        let command_id = Uuid::new_v4();
        let command = robot_fleet_common::types::RobotCommandMessage {
            command_id,
            robot_id: "robot-01".into(),
            command_type: "extreme_temperature".into(),
            payload: json!({}),
            expires_at: None,
        };

        let message = sensor_event_message("robot-01", &command, "extreme_temperature", "high");

        assert_eq!(message.command_id, None);
        assert_eq!(message.payload["simulation_request_id"], json!(command_id));
    }

    #[test]
    fn startup_recovery_payload_includes_restart_reason() {
        let record = sample_record();
        let payload = startup_recovery_payload(&record);

        assert_eq!(payload["command_type"], json!("move"));
        assert_eq!(
            payload["payload"],
            json!({ "target_position_x": 1.0, "target_position_y": 2.0 })
        );
        assert_eq!(payload["reason"], json!(STARTUP_RECOVERY_REASON));
    }

    #[tokio::test]
    async fn concurrent_journal_writes_are_serialized() {
        let journal_lock = Arc::new(Mutex::new(()));
        let path = std::env::temp_dir().join(format!(
            "robot-simulator-journal-race-{}.json",
            Uuid::new_v4()
        ));

        let command_a = robot_fleet_common::types::RobotCommandMessage {
            command_id: Uuid::new_v4(),
            robot_id: "robot-01".into(),
            command_type: "move".into(),
            payload: json!({ "target_position_x": 1.0, "target_position_y": 2.0 }),
            expires_at: None,
        };
        let command_b = robot_fleet_common::types::RobotCommandMessage {
            command_id: Uuid::new_v4(),
            robot_id: "robot-01".into(),
            command_type: "stop".into(),
            payload: json!({ "stop": true }),
            expires_at: None,
        };

        let mut records = HashMap::new();
        records.insert(
            command_a.command_id,
            ProcessedCommandRecord::new(&command_a, ProcessedCommandStatus::Received, Utc::now()),
        );
        records.insert(
            command_b.command_id,
            ProcessedCommandRecord::new(&command_b, ProcessedCommandStatus::Received, Utc::now()),
        );

        let state = Arc::new(Mutex::new(RobotState::new(records, Duration::from_secs(1))));

        let first = update_command_record_status(
            &state,
            &journal_lock,
            &path,
            command_a.command_id,
            ProcessedCommandStatus::Running,
            Utc::now(),
        );
        let second = update_command_record_status(
            &state,
            &journal_lock,
            &path,
            command_b.command_id,
            ProcessedCommandStatus::Completed,
            Utc::now(),
        );
        let (first, second) = tokio::join!(first, second);
        first.expect("first write");
        second.expect("second write");

        let contents = tokio::fs::read_to_string(&path)
            .await
            .expect("read journal");
        let persisted: HashMap<Uuid, ProcessedCommandRecord> =
            serde_json::from_str(&contents).expect("parse journal");
        assert_eq!(
            persisted
                .get(&command_a.command_id)
                .expect("first command")
                .status,
            ProcessedCommandStatus::Running
        );
        assert_eq!(
            persisted
                .get(&command_b.command_id)
                .expect("second command")
                .status,
            ProcessedCommandStatus::Completed
        );

        tokio::fs::remove_file(path).await.expect("remove journal");
    }
}
