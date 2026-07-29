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
    pub(crate) set_velocity: f64,
    pub(crate) velocity: f64,
    pub(crate) direction_degrees: f64,
    pub(crate) stop: bool,
    pub(crate) target_position_x: Option<f64>,
    pub(crate) target_position_y: Option<f64>,
    pub(crate) online: bool,
    pub(crate) current_mission: Option<String>,
    pub(crate) software_version: String,
    pub(crate) processed_commands: HashSet<Uuid>,
    motion_tick_seconds: f64,
}

impl RobotState {
    pub(crate) fn new(processed_commands: HashSet<Uuid>, motion_tick: Duration) -> Self {
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
            software_version: "0.1.0".into(),
            processed_commands,
            motion_tick_seconds,
        }
    }

    pub(crate) fn apply_command(&mut self, command_type: &str, payload: &Value) -> Result<()> {
        match command_type {
            "move" => {
                let (target_position_x, target_position_y) = parse_move_payload(payload)?;
                self.target_position_x = Some(target_position_x);
                self.target_position_y = Some(target_position_y);
                self.stop = false;
                self.current_mission = Some("move".into());
                self.update_motion_state();
            }
            "set_velocity" => {
                let set_velocity = parse_set_velocity_payload(payload)?;
                self.set_velocity = set_velocity;
                self.update_motion_state();
            }
            "stop" => {
                let stop = parse_stop_payload(payload)?;
                self.stop = stop;
                self.update_motion_state();
            }
            _ => {}
        }
        Ok(())
    }

    fn advance_motion(&mut self) {
        if self.stop {
            self.velocity = 0.0;
            return;
        }

        let Some(target_position_x) = self.target_position_x else {
            self.velocity = 0.0;
            return;
        };
        let Some(target_position_y) = self.target_position_y else {
            self.velocity = 0.0;
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
        }
    }

    fn update_motion_state(&mut self) {
        let Some(target_position_x) = self.target_position_x else {
            self.velocity = 0.0;
            return;
        };
        let Some(target_position_y) = self.target_position_y else {
            self.velocity = 0.0;
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
            return;
        }
        self.direction_degrees = delta_y.atan2(delta_x).to_degrees().rem_euclid(360.0);
        self.velocity = if self.stop || self.set_velocity <= 0.0 {
            0.0
        } else {
            self.set_velocity
        };
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
    let target_position_x = payload
        .get("target_position_x")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("move command payload must include numeric target_position_x"))?;
    let target_position_y = payload
        .get("target_position_y")
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("move command payload must include numeric target_position_y"))?;

    Ok((target_position_x, target_position_y))
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
    use std::{collections::HashSet, time::Duration};

    use serde_json::json;
    use uuid::Uuid;

    use super::RobotState;

    #[test]
    fn duplicate_command_is_detected() {
        let command_id = Uuid::new_v4();
        let mut state = RobotState::new(HashSet::new(), Duration::from_secs(1));

        assert!(state.processed_commands.insert(command_id));
        assert!(!state.processed_commands.insert(command_id));
    }

    #[test]
    fn move_command_sets_target_and_uses_velocity() {
        let mut state = RobotState::new(HashSet::new(), Duration::from_secs(1));
        state
            .apply_command("set_velocity", &json!({ "set_velocity": 2 }))
            .expect("set_velocity command");
        state
            .apply_command(
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
        assert_eq!(state.velocity, 2.0);
    }

    #[test]
    fn set_velocity_command_updates_value_and_respects_bounds() {
        let mut state = RobotState::new(HashSet::new(), Duration::from_secs(1));
        state
            .apply_command("set_velocity", &json!({ "set_velocity": 1.5 }))
            .expect("set_velocity command");
        assert_eq!(state.set_velocity, 1.5);
        assert_eq!(state.velocity, 0.0);
    }

    #[test]
    fn stop_command_pauses_and_resumes_motion() {
        let mut state = RobotState::new(HashSet::new(), Duration::from_secs(1));
        state
            .apply_command("set_velocity", &json!({ "set_velocity": 4 }))
            .expect("set_velocity command");
        state
            .apply_command(
                "move",
                &json!({
                    "target_position_x": 10,
                    "target_position_y": 0
                }),
            )
            .expect("move command");

        state
            .apply_command("stop", &json!({ "stop": true }))
            .expect("stop command");
        assert!(state.stop);
        assert_eq!(state.velocity, 0.0);

        state
            .apply_command("stop", &json!({ "stop": false }))
            .expect("resume command");
        assert!(!state.stop);
        assert_eq!(state.velocity, 4.0);
    }

    #[test]
    fn motion_advance_moves_toward_target() {
        let mut state = RobotState::new(HashSet::new(), Duration::from_secs(1));
        state
            .apply_command("set_velocity", &json!({ "set_velocity": 1 }))
            .expect("set_velocity command");
        state
            .apply_command(
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
    }
}
