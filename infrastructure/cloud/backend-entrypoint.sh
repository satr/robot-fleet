#!/bin/sh
set -eu

DATA_DIR=/var/lib/postgresql/data
mkdir -p "$DATA_DIR"
chown -R postgres:postgres "$DATA_DIR"
if [ ! -s "$DATA_DIR/PG_VERSION" ]; then
  su -s /bin/sh postgres -c "/usr/lib/postgresql/15/bin/initdb -D $DATA_DIR"
fi
su -s /bin/sh postgres -c "/usr/lib/postgresql/15/bin/pg_ctl -D $DATA_DIR -o '-c listen_addresses=127.0.0.1' -w start"
until su -s /bin/sh postgres -c "pg_isready -h 127.0.0.1" >/dev/null 2>&1; do sleep 1; done
su -s /bin/sh postgres -c "psql -v ON_ERROR_STOP=1 --dbname=postgres" <<'SQL'
DO $$ BEGIN
  CREATE ROLE robot_fleet LOGIN PASSWORD 'robot_fleet';
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
SQL
su -s /bin/sh postgres -c "createdb -O robot_fleet robot_fleet" 2>/dev/null || true
mosquitto -c /etc/mosquitto/mosquitto.conf &
exec /usr/local/bin/robot-fleet-backend
