use std::{sync::Arc, time::Duration};

use chrono::Utc;
use rand::Rng;
use robot_fleet_common::{
    mqtt::parse_mqtt_url,
    types::{CommandResultMessage, RobotCommandMessage, StateMessage, TelemetryMessage},
};
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::json;
use tokio::{
    sync::Mutex,
    time::{interval, sleep, MissedTickBehavior},
};
use tracing::{info, warn};

use crate::{
    config::Config, metrics::Metrics, persistence::persist_processed_command, state::RobotState,
};

pub(crate) async fn run_robot(
    config: Config,
    state: Arc<Mutex<RobotState>>,
    metrics: Arc<Metrics>,
) -> anyhow::Result<()> {
    loop {
        let (client, mut eventloop) = connect_mqtt(&config).await?;
        client
            .subscribe(
                format!("robots/{}/commands", config.robot_id),
                QoS::AtLeastOnce,
            )
            .await?;
        publish_state(&client, &config, &state).await?;

        let mut telemetry_interval = interval(config.telemetry_interval);
        telemetry_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                poll_result = eventloop.poll() => {
                    match poll_result {
                        Ok(Event::Incoming(Incoming::Publish(publish))) => {
                            metrics.mqtt_connection_status.set(1.0);
                            if let Err(err) =
                                handle_command(&client, &config, &state, &metrics, &publish.payload).await
                            {
                                warn!(robot_id = config.robot_id, error = %err, "command handling failed");
                            }
                        }
                        Ok(_) => metrics.mqtt_connection_status.set(1.0),
                        Err(err) => {
                            metrics.mqtt_connection_status.set(0.0);
                            warn!(robot_id = config.robot_id, error = %err, "MQTT disconnected; reconnecting");
                            sleep(Duration::from_secs(2)).await;
                            break;
                        }
                    }
                }
                _ = telemetry_interval.tick() => {
                    if let Err(err) = publish_telemetry(&client, &config, &state, &metrics).await {
                        metrics.mqtt_connection_status.set(0.0);
                        warn!(robot_id = config.robot_id, error = %err, "telemetry publish failed; reconnecting");
                        sleep(Duration::from_secs(2)).await;
                        break;
                    }
                }
            }
        }
    }
}

async fn connect_mqtt(config: &Config) -> anyhow::Result<(AsyncClient, rumqttc::EventLoop)> {
    let (host, port) = parse_mqtt_url(&config.mqtt_url)?;
    let mut options = MqttOptions::new(&config.mqtt_client_id, host, port);
    options.set_keep_alive(Duration::from_secs(10));
    Ok(AsyncClient::new(options, 10))
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
        velocity_cm_s: guard.velocity_cm_s,
        direction_degrees: guard.direction_degrees,
        payload: json!({ "online": guard.online }),
    };
    let state_message = StateMessage {
        robot_id: config.robot_id.clone(),
        name: config.robot_name.clone(),
        status: if guard.online { "online" } else { "offline" }.into(),
        battery_level: guard.battery_level,
        position_x: guard.position_x,
        position_y: guard.position_y,
        velocity_cm_s: guard.velocity_cm_s,
        direction_degrees: guard.direction_degrees,
        current_mission: guard.current_mission.clone(),
        software_version: guard.software_version.clone(),
        recorded_at: now,
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
    client
        .publish(
            format!("robots/{}/state", config.robot_id),
            QoS::AtLeastOnce,
            false,
            serde_json::to_vec(&state_message)?,
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
    let guard = state.lock().await;
    let message = StateMessage {
        robot_id: config.robot_id.clone(),
        name: config.robot_name.clone(),
        status: if guard.online { "online" } else { "offline" }.into(),
        battery_level: guard.battery_level,
        position_x: guard.position_x,
        position_y: guard.position_y,
        velocity_cm_s: guard.velocity_cm_s,
        direction_degrees: guard.direction_degrees,
        current_mission: guard.current_mission.clone(),
        software_version: guard.software_version.clone(),
        recorded_at: Utc::now(),
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
    if command.robot_id != config.robot_id {
        warn!(
            robot_id = config.robot_id,
            command_robot_id = command.robot_id,
            command_id = %command.command_id,
            "ignoring command for a different robot"
        );
        return Ok(());
    }

    let mut guard = state.lock().await;
    if guard.processed_commands.contains(&command.command_id) {
        info!(robot_id = config.robot_id, command_id = %command.command_id, "duplicate command ignored");
        return Ok(());
    }
    persist_processed_command(&config.processed_commands_path, command.command_id).await?;
    guard.processed_commands.insert(command.command_id);
    guard.current_mission = Some(command.command_type.clone());
    guard.apply_command(&command.command_type, &command.payload)?;
    drop(guard);

    metrics.commands_processed.inc();
    publish_command_result(
        client,
        config,
        &command,
        "acknowledged",
        "command_acknowledged",
    )
    .await?;
    publish_command_result(client, config, &command, "running", "command_running").await?;
    publish_telemetry(client, config, state, metrics).await?;
    sleep(Duration::from_secs(2)).await;
    {
        let mut guard = state.lock().await;
        guard.current_mission = None;
    }
    publish_state(client, config, state).await?;
    publish_command_result(client, config, &command, "completed", "command_completed").await?;
    Ok(())
}

async fn publish_command_result(
    client: &AsyncClient,
    config: &Config,
    command: &RobotCommandMessage,
    status: &str,
    event_type: &str,
) -> anyhow::Result<()> {
    let message = CommandResultMessage {
        command_id: command.command_id,
        robot_id: config.robot_id.clone(),
        status: status.into(),
        event_type: event_type.into(),
        payload: json!({
            "command_type": command.command_type,
            "payload": command.payload,
        }),
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
