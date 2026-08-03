use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::{
    sync::Mutex,
    time::{interval, MissedTickBehavior},
};
use uuid::Uuid;

use crate::persistence::ProcessedCommandRecord;

#[derive(Debug)]
pub(crate) struct RobotState {
    pub(crate) battery_level: f64,
    pub(crate) position_x: f64,
    pub(crate) position_y: f64,
    pub(crate) set_velocity: f64,
    pub(crate) velocity: f64,
    pub(crate) direction_degrees: f64,
    pub(crate) stop: bool,
    pub(crate) target_position_x: Option<f64>,
    pub(crate) target_position_y: Option<f64>,
    pub(crate) online: bool,
    pub(crate) current_mission: Option<String>,
    pub(crate) state: String,
    pub(crate) software_version: String,
    pub(crate) processed_commands: HashMap<Uuid, ProcessedCommandRecord>,
    processing_commands: HashSet<Uuid>,
    pub(crate) current_move_command_id: Option<Uuid>,
    completed_move_commands: HashSet<Uuid>,
    safe_state: bool,
    motion_tick_seconds: f64,
}

#[derive(Debug, PartialEq)]
pub(crate) enum AppliedCommand {
    Move {
        overridden_command_id: Option<Uuid>,
    },
    SetVelocity,
    Stop {
        stop: bool,
        affected_move_command_id: Option<Uuid>,
    },
    SimulateEvent {
        event_type: String,
        priority: String,
        interrupted_move_command_id: Option<Uuid>,
    },
}

impl RobotState {
    pub(crate) fn new(
        processed_commands: HashMap<Uuid, ProcessedCommandRecord>,
        motion_tick: Duration,
    ) -> Self {
        let motion_tick_seconds = motion_tick.as_secs_f64().max(f64::EPSILON);
        Self {
            battery_level: 100.0,
            position_x: 0.0,
            position_y: 0.0,
            set_velocity: 1.0,
            velocity: 0.0,
            direction_degrees: 0.0,
            stop: false,
            target_position_x: None,
            target_position_y: None,
            online: true,
            current_mission: None,
            state: "idle".into(),
            software_version: "0.1.0".into(),
            processed_commands,
            processing_commands: HashSet::new(),
            current_move_command_id: None,
            completed_move_commands: HashSet::new(),
            safe_state: false,
            motion_tick_seconds,
        }
    }

    pub(crate) fn processed_command(&self, command_id: &Uuid) -> Option<&ProcessedCommandRecord> {
        self.processed_commands.get(command_id)
    }

    pub(crate) fn remember_processed_command(&mut self, record: ProcessedCommandRecord) {
        self.processed_commands.insert(record.command_id, record);
    }

    pub(crate) fn start_processing_command(&mut self, command_id: Uuid) -> bool {
        self.processing_commands.insert(command_id)
    }

    pub(crate) fn finish_processing_command(&mut self, command_id: Uuid) {
        self.processing_commands.remove(&command_id);
    }

    pub(crate) fn command_is_processing(&self, command_id: Uuid) -> bool {
        self.processing_commands.contains(&command_id)
    }

    pub(crate) fn apply_command(
        &mut self,
        command_id: Uuid,
        command_type: &str,
        payload: &Value,
    ) -> Result<AppliedCommand> {
        let normalized_command_type = normalize_command_type(command_type);
        match normalized_command_type.as_str() {
            "move" => {
                let (target_position_x, target_position_y) = parse_move_payload(payload)?;
                let overridden_command_id = self
                    .current_move_command_id
                    .filter(|current| *current != command_id);
                self.target_position_x = Some(target_position_x);
                self.target_position_y = Some(target_position_y);
                self.stop = false;
                self.current_mission = Some("move".into());
                self.safe_state = false;
                self.current_move_command_id = Some(command_id);
                self.update_motion_state();
                Ok(AppliedCommand::Move {
                    overridden_command_id,
                })
            }
            "set_velocity" => {
                let set_velocity = parse_set_velocity_payload(payload)?;
                self.set_velocity = set_velocity;
                self.update_motion_state();
                Ok(AppliedCommand::SetVelocity)
            }
            "stop" => {
                let stop = parse_stop_payload(payload)?;
                self.stop = stop;
                if !stop {
                    self.safe_state = false;
                }
                self.update_motion_state();
                Ok(AppliedCommand::Stop {
                    stop,
                    affected_move_command_id: self.current_move_command_id,
                })
            }
            other => Err(anyhow!("unsupported command_type: {other}")),
        }
    }

    pub(crate) fn finish_safe_state(&mut self) {
        self.current_mission = None;
        self.safe_state = true;
        self.refresh_operating_state();
    }

    pub(crate) fn take_completed_move_command(&mut self, command_id: Uuid) -> bool {
        self.completed_move_commands.remove(&command_id)
    }

    pub(crate) fn move_command_is_active(&self, command_id: Uuid) -> bool {
        self.current_move_command_id == Some(command_id)
    }

    fn advance_motion(&mut self) {
        if self.stop {
            self.velocity = 0.0;
            self.refresh_operating_state();
            return;
        }

        let Some(target_position_x) = self.target_position_x else {
            self.velocity = 0.0;
            self.refresh_operating_state();
            return;
        };
        let Some(target_position_y) = self.target_position_y else {
            self.velocity = 0.0;
            self.refresh_operating_state();
            return;
        };

        let delta_x = target_position_x - self.position_x;
        let delta_y = target_position_y - self.position_y;
        let distance = (delta_x.powi(2) + delta_y.powi(2)).sqrt();
        if distance <= f64::EPSILON || self.set_velocity <= 0.0 {
            self.position_x = target_position_x;
            self.position_y = target_position_y;
            self.velocity = 0.0;
            self.target_position_x = None;
            self.target_position_y = None;
            self.current_mission = None;
            self.complete_current_move_command();
            self.refresh_operating_state();
            return;
        }

        let max_step = self.set_velocity * self.motion_tick_seconds;
        let step = distance.min(max_step);
        let scale = step / distance;
        self.position_x += delta_x * scale;
        self.position_y += delta_y * scale;
        self.velocity = step / self.motion_tick_seconds;
        self.direction_degrees = delta_y.atan2(delta_x).to_degrees().rem_euclid(360.0);

        if step >= distance {
            self.position_x = target_position_x;
            self.position_y = target_position_y;
            self.velocity = 0.0;
            self.target_position_x = None;
            self.target_position_y = None;
            self.current_mission = None;
            self.complete_current_move_command();
        } else {
            self.refresh_operating_state();
        }
    }

    fn update_motion_state(&mut self) {
        let Some(target_position_x) = self.target_position_x else {
            self.velocity = 0.0;
            self.refresh_operating_state();
            return;
        };
        let Some(target_position_y) = self.target_position_y else {
            self.velocity = 0.0;
            self.refresh_operating_state();
            return;
        };

        let delta_x = target_position_x - self.position_x;
        let delta_y = target_position_y - self.position_y;
        let distance = (delta_x.powi(2) + delta_y.powi(2)).sqrt();
        if distance <= f64::EPSILON {
            self.velocity = 0.0;
            self.target_position_x = None;
            self.target_position_y = None;
            self.current_mission = None;
            self.complete_current_move_command();
            self.refresh_operating_state();
            return;
        }
        self.direction_degrees = delta_y.atan2(delta_x).to_degrees().rem_euclid(360.0);
        self.velocity = if self.stop || self.set_velocity <= 0.0 {
            0.0
        } else {
            self.set_velocity
        };
        self.refresh_operating_state();
    }

    fn complete_current_move_command(&mut self) {
        if let Some(command_id) = self.current_move_command_id.take() {
            self.completed_move_commands.insert(command_id);
        }
        self.refresh_operating_state();
    }

    pub(crate) fn start_simulated_event(
        &mut self,
        event_type: &str,
        priority: &str,
    ) -> AppliedCommand {
        let interrupted_move_command_id = self.current_move_command_id.take();
        self.target_position_x = None;
        self.target_position_y = None;
        self.velocity = 0.0;
        self.stop = true;
        self.safe_state = false;
        self.current_mission = Some("get_to_save_state".into());
        self.refresh_operating_state();
        AppliedCommand::SimulateEvent {
            event_type: event_type.into(),
            priority: priority.into(),
            interrupted_move_command_id,
        }
    }

    fn refresh_operating_state(&mut self) {
        if self.safe_state {
            self.state = "idle in safe state".into();
            return;
        }
        if let Some(current_mission) = &self.current_mission {
            self.state = format!("running: {current_mission}");
            return;
        }
        self.state = "idle".into();
    }

    pub(crate) async fn update_state(state: Arc<Mutex<Self>>, interval_duration: Duration) {
        let mut state_interval = interval(interval_duration);
        state_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            state_interval.tick().await;
            let mut guard = state.lock().await;
            guard.advance_motion();
        }
    }
}

fn parse_move_payload(payload: &Value) -> Result<(f64, f64)> {
    if let Some(target_position) = payload.get("target_position") {
        if let Some(position) = target_position.as_array() {
            if position.len() == 2 {
                let target_position_x = position[0]
                    .as_f64()
                    .ok_or_else(|| anyhow!("target_position[0] must be numeric"))?;
                let target_position_y = position[1]
                    .as_f64()
                    .ok_or_else(|| anyhow!("target_position[1] must be numeric"))?;
                return Ok((target_position_x, target_position_y));
            }
        }

        if let Some(position) = target_position.as_object() {
            let target_position_x = position
                .get("x")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("target_position.x must be numeric"))?;
            let target_position_y = position
                .get("y")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("target_position.y must be numeric"))?;
            return Ok((target_position_x, target_position_y));
        }
    }

    let target_position_x = payload
        .get("target_position_x")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            anyhow!(
                "move command payload must include numeric target_position_x/target_position_y or target_position"
            )
        })?;
    let target_position_y = payload
        .get("target_position_y")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            anyhow!(
                "move command payload must include numeric target_position_x/target_position_y or target_position"
            )
        })?;

    Ok((target_position_x, target_position_y))
}

fn normalize_command_type(command_type: &str) -> String {
    command_type
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

fn parse_set_velocity_payload(payload: &Value) -> Result<f64> {
    let set_velocity = payload
        .get("set_velocity")
        .and_then(Value::as_f64)
        .unwrap_or(1.0);

    if !(0.01..=10.0).contains(&set_velocity) {
        return Err(anyhow!("set_velocity must be between 0.01 and 10.0"));
    }

    Ok(set_velocity)
}

fn parse_stop_payload(payload: &Value) -> Result<bool> {
    payload
        .as_bool()
        .or_else(|| payload.get("stop").and_then(Value::as_bool))
        .ok_or_else(|| anyhow!("stop command payload must include boolean stop"))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use serde_json::json;
    use uuid::Uuid;

    use super::{AppliedCommand, RobotState};
    use crate::persistence::{ProcessedCommandRecord, ProcessedCommandStatus};

    #[test]
    fn duplicate_command_is_detected() {
        let command_id = Uuid::new_v4();
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));

        assert!(state
            .processed_commands
            .insert(
                command_id,
                ProcessedCommandRecord {
                    command_id,
                    command_type: "move".into(),
                    payload: json!({}),
                    status: ProcessedCommandStatus::Completed,
                    updated_at: chrono::Utc::now(),
                    expires_at: None,
                    acknowledged_at: None,
                    stopped_at: None,
                    resumed_at: None,
                    cancelled_at: None,
                    completed_at: None,
                    failed_at: None,
                }
            )
            .is_none());
        assert!(state.processed_commands.contains_key(&command_id));
    }

    #[test]
    fn move_command_sets_target_and_uses_velocity() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        state
            .apply_command(
                Uuid::new_v4(),
                "set_velocity",
                &json!({ "set_velocity": 2 }),
            )
            .expect("set_velocity command");
        state
            .apply_command(
                Uuid::new_v4(),
                "move",
                &json!({
                    "target_position_x": 10,
                    "target_position_y": 5
                }),
            )
            .expect("move command");

        assert_eq!(state.target_position_x, Some(10.0));
        assert_eq!(state.target_position_y, Some(5.0));
        assert_eq!(state.set_velocity, 2.0);
        assert!(!state.stop);
        assert_eq!(state.current_mission.as_deref(), Some("move"));
        assert_eq!(state.state, "running: move");
        assert_eq!(state.velocity, 2.0);
    }

    #[test]
    fn move_command_accepts_target_position_object() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        state
            .apply_command(
                Uuid::new_v4(),
                "move",
                &json!({ "target_position": { "x": 3, "y": 4 } }),
            )
            .expect("move command");

        assert_eq!(state.target_position_x, Some(3.0));
        assert_eq!(state.target_position_y, Some(4.0));
    }

    #[test]
    fn new_move_command_overrides_active_move() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        let first_command_id = Uuid::new_v4();
        let second_command_id = Uuid::new_v4();
        state
            .apply_command(
                first_command_id,
                "move",
                &json!({ "target_position_x": 10, "target_position_y": 0 }),
            )
            .expect("first move command");

        let applied = state
            .apply_command(
                second_command_id,
                "move",
                &json!({ "target_position_x": 20, "target_position_y": 0 }),
            )
            .expect("second move command");

        assert_eq!(
            applied,
            AppliedCommand::Move {
                overridden_command_id: Some(first_command_id)
            }
        );
        assert_eq!(state.current_move_command_id, Some(second_command_id));
        assert_eq!(state.target_position_x, Some(20.0));
    }

    #[test]
    fn set_velocity_command_updates_value_and_respects_bounds() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        state
            .apply_command(
                Uuid::new_v4(),
                "set_velocity",
                &json!({ "set_velocity": 1.5 }),
            )
            .expect("set_velocity command");
        assert_eq!(state.set_velocity, 1.5);
        assert_eq!(state.velocity, 0.0);
    }

    #[test]
    fn stop_command_pauses_and_resumes_motion() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        state
            .apply_command(
                Uuid::new_v4(),
                "set_velocity",
                &json!({ "set_velocity": 4 }),
            )
            .expect("set_velocity command");
        state
            .apply_command(
                Uuid::new_v4(),
                "move",
                &json!({
                    "target_position_x": 10,
                    "target_position_y": 0
                }),
            )
            .expect("move command");

        state
            .apply_command(Uuid::new_v4(), "stop", &json!({ "stop": true }))
            .expect("stop command");
        assert!(state.stop);
        assert_eq!(state.velocity, 0.0);

        state
            .apply_command(Uuid::new_v4(), "stop", &json!({ "stop": false }))
            .expect("resume command");
        assert!(!state.stop);
        assert_eq!(state.velocity, 4.0);
    }

    #[test]
    fn motion_advance_moves_toward_target() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        state
            .apply_command(
                Uuid::new_v4(),
                "set_velocity",
                &json!({ "set_velocity": 1 }),
            )
            .expect("set_velocity command");
        state
            .apply_command(
                Uuid::new_v4(),
                "move",
                &json!({
                    "target_position_x": 2,
                    "target_position_y": 0
                }),
            )
            .expect("move command");

        state.advance_motion();
        assert_eq!(state.position_x, 1.0);
        assert_eq!(state.position_y, 0.0);
        assert_eq!(state.velocity, 1.0);

        state.advance_motion();
        assert_eq!(state.position_x, 2.0);
        assert_eq!(state.position_y, 0.0);
        assert_eq!(state.velocity, 0.0);
        assert_eq!(state.target_position_x, None);
        assert_eq!(state.target_position_y, None);
        assert_eq!(state.state, "idle");
    }

    #[test]
    fn start_simulated_event_interrupts_motion_and_finishes_safe_state() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));
        let move_command_id = Uuid::new_v4();
        state
            .apply_command(
                move_command_id,
                "move",
                &json!({ "target_position_x": 10, "target_position_y": 0 }),
            )
            .expect("move command");

        let applied = state.start_simulated_event("extreme_temperature", "high");

        assert_eq!(
            applied,
            AppliedCommand::SimulateEvent {
                event_type: "extreme_temperature".into(),
                priority: "high".into(),
                interrupted_move_command_id: Some(move_command_id)
            }
        );
        assert_eq!(state.velocity, 0.0);
        assert!(state.stop);
        assert_eq!(state.current_mission.as_deref(), Some("get_to_save_state"));
        assert_eq!(state.state, "running: get_to_save_state");

        state.finish_safe_state();

        assert_eq!(state.current_mission, None);
        assert_eq!(state.state, "idle in safe state");
    }

    #[test]
    fn apply_command_rejects_simulated_event_commands() {
        let mut state = RobotState::new(HashMap::new(), Duration::from_secs(1));

        let err = state
            .apply_command(Uuid::new_v4(), "extreme_temperature", &json!({}))
            .expect_err("simulated event command");

        assert!(err.to_string().contains("unsupported command_type"));
    }
}
