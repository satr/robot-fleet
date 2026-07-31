ALTER TABLE commands
    ADD CONSTRAINT commands_status_check
    CHECK (status IN ('created', 'acknowledged', 'running', 'completed', 'failed', 'expired', 'stopped'));
