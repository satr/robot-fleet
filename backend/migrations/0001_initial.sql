CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE IF NOT EXISTS robots (
    robot_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'offline',
    battery_level DOUBLE PRECISION NOT NULL DEFAULT 100,
    current_mission TEXT,
    last_seen_at TIMESTAMPTZ,
    software_version TEXT NOT NULL DEFAULT 'unknown',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS commands (
    command_id UUID PRIMARY KEY,
    robot_id TEXT NOT NULL REFERENCES robots(robot_id) ON DELETE CASCADE,
    command_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'created',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ,
    acknowledged_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS commands_robot_id_created_at_idx
    ON commands(robot_id, created_at DESC);

CREATE TABLE IF NOT EXISTS command_events (
    event_id UUID PRIMARY KEY,
    command_id UUID NOT NULL REFERENCES commands(command_id) ON DELETE CASCADE,
    robot_id TEXT NOT NULL REFERENCES robots(robot_id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS command_events_command_id_occurred_at_idx
    ON command_events(command_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS telemetry (
    robot_id TEXT NOT NULL REFERENCES robots(robot_id) ON DELETE CASCADE,
    recorded_at TIMESTAMPTZ NOT NULL,
    battery_level DOUBLE PRECISION NOT NULL,
    temperature DOUBLE PRECISION NOT NULL,
    position_x DOUBLE PRECISION NOT NULL,
    position_y DOUBLE PRECISION NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (robot_id, recorded_at)
);

SELECT create_hypertable('telemetry', 'recorded_at', if_not_exists => TRUE);
