use std::collections::HashSet;

use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct RobotState {
    pub(crate) battery_level: f64,
    pub(crate) position_x: f64,
    pub(crate) position_y: f64,
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
            online: true,
            current_mission: None,
            software_version: "0.1.0".into(),
            processed_commands,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use uuid::Uuid;

    use super::RobotState;

    #[test]
    fn duplicate_command_is_detected() {
        let command_id = Uuid::new_v4();
        let mut state = RobotState::new(HashSet::new());

        assert!(state.processed_commands.insert(command_id));
        assert!(!state.processed_commands.insert(command_id));
    }
}
