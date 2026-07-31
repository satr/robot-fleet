# Scope

Robot Fleet is a local prototype for learning robot-fleet control loops. It includes:

- a Rust backend;
- MQTT-based robot simulators;
- PostgreSQL plus TimescaleDB persistence;
- Prometheus, VictoriaMetrics, Grafana, vmalert, and Alertmanager for observability and alerting;
- a SvelteKit operator shell.

Current design choices:

- commands are assigned UUIDs before publish and processed idempotently on the robot side;
- robot status is derived from `last_seen_at`, not from a robot-reported status string;
- telemetry history lives in a TimescaleDB hypertable;
- the web app is an operator shell, not a production UI;
- authentication, TLS, and production hardening are intentionally out of scope for now.

This repository is meant to stay small and understandable so it can be extended step by step.
