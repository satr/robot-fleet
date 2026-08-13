#!/bin/sh
set -eu

test -n "${MQTT_USERNAME:-}" || {
  echo "MQTT_USERNAME is required" >&2
  exit 1
}
test -n "${MQTT_PASSWORD:-}" || {
  echo "MQTT_PASSWORD is required" >&2
  exit 1
}

mkdir -p /mosquitto/data
mosquitto_passwd -b -c /mosquitto/data/password_file "$MQTT_USERNAME" "$MQTT_PASSWORD"
chown mosquitto:mosquitto /mosquitto/data /mosquitto/data/password_file
exec mosquitto -c /mosquitto/config/mosquitto.conf
