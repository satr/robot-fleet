ALTER TABLE robots
    ADD COLUMN IF NOT EXISTS set_velocity DOUBLE PRECISION NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS velocity DOUBLE PRECISION NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'robots'
          AND column_name = 'speed_cm_s'
    ) THEN
        EXECUTE 'UPDATE robots
                 SET set_velocity = COALESCE(set_velocity, speed_cm_s, 1),
                     velocity = COALESCE(velocity, velocity_cm_s, 0)';
        EXECUTE 'ALTER TABLE robots
                 DROP COLUMN IF EXISTS speed_cm_s,
                 DROP COLUMN IF EXISTS velocity_cm_s';
    END IF;
END $$;
