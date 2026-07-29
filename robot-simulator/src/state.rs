use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::{
    sync::Mutex,
    time::{interval, MissedTickBehavior},
};
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct RobotState {
    pub(crate) battery_level: f64,
    pub(crate) position_x: f64,
    pub(crate) position_y: f64,
    pub(crate) motion_x: f64,
    pub(crate) motion_y: f64,
    pub(crate) is_running: bool,
    pub(crate) online: bool,
    pub(crate) current_mission: Option<String>,
    pub(crate) software_version: String,
    pub(crate) processed_commands: HashSet<Uuid>,
}

impl RobotState {
    pub(crate) fn new(processed_commands: HashSet<Uuid>) -> Self {
        Self {
            battery_level: 100.0,
            position_x: 0.0,
            position_y: 0.0,
            motion_x: 0.0,
            motion_y: 0.0,
            is_running: false,
            online: true,
            current_mission: None,
            software_version: "0.1.0".into(),
            processed_commands,
        }
    }

    pub(crate) fn apply_command(&mut self, command_type: &str, payload: &Value) -> Result<()> {
        match command_type {
            "move" => {
                let (motion_x, motion_y) = parse_move_payload(payload)?;
                self.position_x += motion_x;
                self.position_y += motion_y;
                self.motion_x = motion_x;
                self.motion_y = motion_y;
            }
            "run" => {
                if self.motion_x == 0.0 && self.motion_y == 0.0 {
                    self.motion_x = 1.0;
                    self.motion_y = 0.0;
                }
                self.is_running = true;
            }
            "stop" => {
                self.is_running = false;
            }
            _ => {}
        }
        Ok(())
    }

    fn advance_motion(&mut self) {
        if self.is_running {
            self.position_x += self.motion_x;
            self.position_y += self.motion_y;
        }
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
    let axis = payload
        .get("axis")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("move command payload must include axis"))?;
    let delta = payload
        .get("delta")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("move command payload must include numeric delta"))?;

    if delta == 0.0 {
        return Err(anyhow!("move command delta must not be zero"));
    }

    match axis {
        "x" => Ok((delta, 0.0)),
        "y" => Ok((0.0, delta)),
        other => Err(anyhow!("unsupported move axis: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::json;
    use uuid::Uuid;

    use super::RobotState;

    #[test]
    fn duplicate_command_is_detected() {
        let command_id = Uuid::new_v4();
        let mut state = RobotState::new(HashSet::new());

        assert!(state.processed_commands.insert(command_id));
        assert!(!state.processed_commands.insert(command_id));
    }

    #[test]
    fn move_command_updates_position_and_motion() {
        let mut state = RobotState::new(HashSet::new());

        state
            .apply_command("move", &json!({ "axis": "x", "delta": 1 }))
            .expect("move command");

        assert_eq!(state.position_x, 1.0);
        assert_eq!(state.position_y, 0.0);
        assert_eq!(state.motion_x, 1.0);
        assert_eq!(state.motion_y, 0.0);
        assert!(!state.is_running);
    }

    #[test]
    fn run_command_advances_using_last_motion() {
        let mut state = RobotState::new(HashSet::new());

        state
            .apply_command("move", &json!({ "axis": "y", "delta": -1 }))
            .expect("move command");
        state.apply_command("run", &json!({})).expect("run command");

        assert!(state.is_running);
        assert_eq!(state.position_x, 0.0);
        assert_eq!(state.position_y, -1.0);

        state.advance_motion();
        assert_eq!(state.position_y, -2.0);
    }

    #[test]
    fn stop_command_halts_motion() {
        let mut state = RobotState::new(HashSet::new());

        state
            .apply_command("move", &json!({ "axis": "x", "delta": 1 }))
            .expect("move command");
        state.apply_command("run", &json!({})).expect("run command");
        state.apply_command("stop", &json!({})).expect("stop command");

        let position_x = state.position_x;
        state.advance_motion();

        assert!(!state.is_running);
        assert_eq!(state.position_x, position_x);
    }
}
