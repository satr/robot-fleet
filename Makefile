.DEFAULT_GOAL := help

ROBOT_ID ?= robot-local
ROBOT_NAME ?= Local Robot
TELEMETRY_INTERVAL_SECONDS ?= 5
MQTT_URL ?= mqtt://localhost:1883
DATABASE_URL ?= postgres://robot_fleet:robot_fleet@localhost:5432/robot_fleet
KAFKA_BROKERS ?= localhost:9092
RUST_LOG ?= info

.PHONY: help infra-up infra-down infra-logs db-migrate backend-run backend-test robot-run dev build test docker-build docker-up docker-down docker-logs clean

help:
	@echo "Robot Fleet commands:"
	@echo "  make infra-up       Start Postgres/TimescaleDB, MQTT, Kafka, Prometheus, Grafana"
	@echo "  make infra-down     Stop infrastructure services"
	@echo "  make infra-logs     Follow infrastructure logs"
	@echo "  make db-migrate     Run backend SQLx migrations"
	@echo "  make backend-run    Run backend locally"
	@echo "  make backend-test   Run backend tests"
	@echo "  make robot-run      Run one simulator locally (set ROBOT_ID=robot-local)"
	@echo "  make dev            Start infrastructure, then run backend locally"
	@echo "  make build          Build Rust workspace"
	@echo "  make test           Run Rust tests"
	@echo "  make docker-build   Build all Docker images"
	@echo "  make docker-up      Build and start the full platform"
	@echo "  make docker-down    Stop the full platform"
	@echo "  make docker-logs    Follow all Docker logs"
	@echo "  make clean          Remove build artifacts"

infra-up:
	docker compose up -d postgres-timescaledb mqtt kafka kafka-init prometheus grafana

infra-down:
	docker compose stop postgres-timescaledb mqtt kafka prometheus grafana

infra-logs:
	docker compose logs -f postgres-timescaledb mqtt kafka kafka-init prometheus grafana

db-migrate:
	docker compose up -d postgres-timescaledb
	@until docker compose exec -T postgres-timescaledb pg_isready -U "$${POSTGRES_USER:-robot_fleet}" -d "$${POSTGRES_DB:-robot_fleet}" >/dev/null 2>&1; do \
		sleep 1; \
	done
	cd backend && DATABASE_URL="$(DATABASE_URL)" cargo sqlx migrate run

backend-run:
	DATABASE_URL="$(DATABASE_URL)" MQTT_URL="$(MQTT_URL)" KAFKA_BROKERS="$(KAFKA_BROKERS)" RUST_LOG="$(RUST_LOG)" cargo run -p robot-fleet-backend

backend-test:
	cargo test -p robot-fleet-backend

robot-run:
	ROBOT_ID="$(ROBOT_ID)" ROBOT_NAME="$(ROBOT_NAME)" MQTT_URL="$(MQTT_URL)" TELEMETRY_INTERVAL_SECONDS="$(TELEMETRY_INTERVAL_SECONDS)" RUST_LOG="$(RUST_LOG)" cargo run -p robot-simulator

dev: infra-up backend-run

build:
	cargo build --workspace

test:
	cargo test --workspace

docker-build:
	docker compose build

docker-up:
	docker compose up --build -d

docker-down:
	docker compose down

docker-logs:
	docker compose logs -f

clean:
	cargo clean
