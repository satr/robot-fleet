export type RobotStatus = 'online' | 'stale' | 'offline';

export type Robot = {
  robot_id: string;
  name: string;
  status: RobotStatus;
  battery_level: number;
  position_x: number | null;
  position_y: number | null;
  set_velocity: number | null;
  velocity: number | null;
  direction_degrees: number | null;
  stop: boolean;
  target_position_x: number | null;
  target_position_y: number | null;
  current_mission: string | null;
  current_command: string | null;
  current_command_status: string | null;
  last_seen_at: string | null;
  software_version: string;
  created_at: string;
  updated_at: string;
};

export type RobotStreamMessage = {
  event_type: 'robot_updated' | 'robot_deleted';
  robot_id: string | null;
  robot: Robot | null;
};
