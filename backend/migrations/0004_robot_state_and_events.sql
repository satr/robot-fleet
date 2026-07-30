ALTER TABLE robots
    ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'idle';

CREATE TABLE IF NOT EXISTS robot_state_history (
    robot_id TEXT NOT NULL REFERENCES robots(robot_id) ON DELETE CASCADE,
    recorded_at TIMESTAMPTZ NOT NULL,
    state TEXT NOT NULL,
    status TEXT NOT NULL,
    battery_level DOUBLE PRECISION NOT NULL,
    position_x DOUBLE PRECISION NOT NULL,
    position_y DOUBLE PRECISION NOT NULL,
    velocity DOUBLE PRECISION NOT NULL,
    current_mission TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (robot_id, recorded_at)
);

SELECT create_hypertable('robot_state_history', 'recorded_at', if_not_exists => TRUE);

CREATE TABLE IF NOT EXISTS robot_sensor_events (
    event_id UUID NOT NULL,
    robot_id TEXT NOT NULL REFERENCES robots(robot_id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    priority TEXT NOT NULL,
    command_id UUID REFERENCES commands(command_id) ON DELETE SET NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (event_id, occurred_at)
);

SELECT create_hypertable('robot_sensor_events', 'occurred_at', if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS robot_sensor_events_robot_id_occurred_at_idx
    ON robot_sensor_events(robot_id, occurred_at DESC);

CREATE INDEX IF NOT EXISTS robot_sensor_events_event_type_occurred_at_idx
    ON robot_sensor_events(event_type, occurred_at DESC);
