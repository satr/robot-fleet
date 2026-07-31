ALTER TABLE commands
    DROP CONSTRAINT IF EXISTS commands_status_check;

ALTER TABLE commands
    ADD CONSTRAINT commands_status_check
    CHECK (status IN ('created', 'publish_failed', 'acknowledged', 'running', 'completed', 'failed', 'expired', 'stopped'));
