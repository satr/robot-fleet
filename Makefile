.DEFAULT_GOAL := help

ROBOT_ID ?= robot-local
ROBOT_NAME ?= Local Robot
TELEMETRY_INTERVAL_SECONDS ?= 5
ROBOT_STATE_INTERVAL_SECONDS ?= 1
MQTT_URL ?= mqtt://localhost:1883
DATABASE_URL ?= postgres://robot_fleet:robot_fleet@localhost:5432/robot_fleet
RUST_LOG ?= info
PROCESSED_COMMANDS_PATH ?= data/robots/$(ROBOT_ID)/processed_commands.json
WEB_PORT ?= 5173
PUBLIC_BACKEND_HTTP_URL ?= http://localhost:8089
PUBLIC_BACKEND_WS_URL ?= ws://localhost:8089
GCP_REGION ?= us-central1
GCP_TEST_PROJECT ?= robot-fleet-test-00000000-0000-0000-0000-000000000001
GCP_PROD_PROJECT ?= robot-fleet-prod-00000000-0000-0000-0000-000000000001
GCP_ENV ?= test
GCP_PROJECT ?= $(if $(filter prod,$(GCP_ENV)),$(GCP_PROD_PROJECT),$(GCP_TEST_PROJECT))
AR_REPOSITORY ?= robot-fleet-$(GCP_ENV)-images
AR_HOST ?= $(GCP_REGION)-docker.pkg.dev
BACKEND_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/backend:latest
WEB_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/web:latest

.PHONY: help infra-up infra-down infra-logs db-migrate backend-run backend-stop-dev backend-test web-run web-stop-dev web-build web-check robot-run robot-dev robot1-run robot2-run robot3-run robots-run robot1-dev robot2-dev robot3-dev robots-dev robots-stop-dev stop-dev robot1-up robot2-up robot3-up robots-up robot1-down robot2-down robot3-down robots-down dev build test docker-prereqs docker-build docker-up docker-down docker-logs cloud-deploy cloud-deploy-test cloud-deploy-prod cloud-start cloud-stop cloud-start-test cloud-stop-test cloud-start-prod cloud-stop-prod clean

help:
	@echo "Robot Fleet commands:"
	@echo "  make infra-up       Start Postgres/TimescaleDB, MQTT, Prometheus, VictoriaMetrics, vmalert, Alertmanager, Grafana"
	@echo "  make infra-down     Stop infrastructure services"
	@echo "  make infra-logs     Follow infrastructure logs"
	@echo "  make db-migrate     Run backend SQLx migrations"
	@echo "  make backend-run    Run backend locally"
	@echo "  make backend-stop-dev  Stop the local backend dev process"
	@echo "  make backend-test   Run backend tests"
	@echo "  make web-run        Run SvelteKit web app locally"
	@echo "  make web-stop-dev     Stop the local web app dev process"
	@echo "  make web-build      Build SvelteKit web app"
	@echo "  make robot-run      Run one simulator locally (set ROBOT_ID=robot-local)"
	@echo "  make robots-run     Run three simulators locally"
	@echo "  make robots-stop-dev  Stop the local robot dev processes"
	@echo "  make stop-dev         Stop local backend, web app, and robot dev processes"
	@echo "  make robot1-down    Stop robot-01 container"
	@echo "  make robot2-down    Stop robot-02 container"
	@echo "  make robot3-down    Stop robot-03 container"
	@echo "  make robots-down    Stop all robot containers"
	@echo "  make dev            Start infrastructure, then run backend locally"
	@echo "  make build          Build Rust workspace"
	@echo "  make test           Run Rust tests"
	@echo "  make docker-build   Build all Docker images"
	@echo "  make docker-up      Build and start the full platform"
	@echo "  make docker-down    Stop the full platform"
	@echo "  make docker-logs    Follow all Docker logs"
	@echo "  make cloud-deploy-test  Build and deploy test to GCP_TEST_PROJECT"
	@echo "  make cloud-deploy-prod  Build and deploy prod to GCP_PROD_PROJECT"
	@echo "  make cloud-start GCP_ENV=test|prod  Restore one Cloud Run instance"
	@echo "  make cloud-stop GCP_ENV=test|prod   Scale Cloud Run services to zero"
	@echo "  make clean          Remove build artifacts"

infra-up:
	docker compose up -d postgres-timescaledb mqtt victoriametrics alertmanager vmalert prometheus grafana

infra-down:
	docker compose stop postgres-timescaledb mqtt victoriametrics alertmanager vmalert prometheus grafana

infra-logs:
	docker compose logs -f postgres-timescaledb mqtt victoriametrics alertmanager vmalert prometheus grafana

db-migrate:
	docker compose up -d postgres-timescaledb
	@until docker compose exec -T postgres-timescaledb pg_isready -U "$${POSTGRES_USER:-robot_fleet}" -d "$${POSTGRES_DB:-robot_fleet}" >/dev/null 2>&1; do \
		sleep 1; \
	done
	cd backend && DATABASE_URL="$(DATABASE_URL)" cargo sqlx migrate run

backend-run:
	DATABASE_URL="$(DATABASE_URL)" MQTT_URL="$(MQTT_URL)" RUST_LOG="$(RUST_LOG)" cargo run -p robot-fleet-backend

backend-stop-dev:
	pkill -f 'robot-fleet-backend'

backend-test:
	cargo test -p robot-fleet-backend

web-run:
	cd web-app && PUBLIC_BACKEND_HTTP_URL="$(PUBLIC_BACKEND_HTTP_URL)" PUBLIC_BACKEND_WS_URL="$(PUBLIC_BACKEND_WS_URL)" WEB_PORT="$(WEB_PORT)" npm run dev

web-stop-dev:
	pkill -f 'npm run dev'

web-build:
	cd web-app && npm run build

web-check:
	cd web-app && npm run check

robot-dev:
	ROBOT_ID="$(ROBOT_ID)" ROBOT_NAME="$(ROBOT_NAME)" MQTT_URL="$(MQTT_URL)" TELEMETRY_INTERVAL_SECONDS="$(TELEMETRY_INTERVAL_SECONDS)" ROBOT_STATE_INTERVAL_SECONDS="$(ROBOT_STATE_INTERVAL_SECONDS)" RUST_LOG="$(RUST_LOG)" PROCESSED_COMMANDS_PATH="$(PROCESSED_COMMANDS_PATH)" cargo run -p robot-simulator

robot1-dev:
	ROBOT_ID="robot-01" ROBOT_NAME="Robot 01" MQTT_URL="$(MQTT_URL)" TELEMETRY_INTERVAL_SECONDS="$(TELEMETRY_INTERVAL_SECONDS)" ROBOT_STATE_INTERVAL_SECONDS="$(ROBOT_STATE_INTERVAL_SECONDS)" RUST_LOG="$(RUST_LOG)" PROCESSED_COMMANDS_PATH="data/robots/robot-01/processed_commands.json" cargo run -p robot-simulator

robot2-dev:
	ROBOT_ID="robot-02" ROBOT_NAME="Robot 02" MQTT_URL="$(MQTT_URL)" TELEMETRY_INTERVAL_SECONDS="$(TELEMETRY_INTERVAL_SECONDS)" ROBOT_STATE_INTERVAL_SECONDS="$(ROBOT_STATE_INTERVAL_SECONDS)" RUST_LOG="$(RUST_LOG)" PROCESSED_COMMANDS_PATH="data/robots/robot-02/processed_commands.json" cargo run -p robot-simulator

robot3-dev:
	ROBOT_ID="robot-03" ROBOT_NAME="Robot 03" MQTT_URL="$(MQTT_URL)" TELEMETRY_INTERVAL_SECONDS="$(TELEMETRY_INTERVAL_SECONDS)" ROBOT_STATE_INTERVAL_SECONDS="$(ROBOT_STATE_INTERVAL_SECONDS)" RUST_LOG="$(RUST_LOG)" PROCESSED_COMMANDS_PATH="data/robots/robot-03/processed_commands.json" cargo run -p robot-simulator

robots-dev:
	$(MAKE) robot1-dev &
	$(MAKE) robot2-dev &
	$(MAKE) robot3-dev &
	wait

robots-stop-dev:
	pkill -f 'robot-simulator'

stop-dev: backend-stop-dev web-stop-dev robots-stop-dev

robot1-up:
	docker compose up -d robot-01

robot2-up:
	docker compose up -d robot-02

robot3-up:
	docker compose up -d robot-03

robots-up:
	docker compose up -d robot-01 robot-02 robot-03

robot1-down:
	docker compose stop robot-01

robot2-down:
	docker compose stop robot-02

robot3-down:
	docker compose stop robot-03

robots-down:
	docker compose stop robot-01 robot-02 robot-03

dev: infra-up
	$(MAKE) backend-run &
	$(MAKE) web-run &
	wait

build:
	cargo build --workspace
	$(MAKE) web-build

test:
	cargo test --workspace
	$(MAKE) web-check

docker-prereqs:
	docker pull rust:1-bookworm
	docker pull debian:bookworm-slim
	docker pull node:22-bookworm-slim

docker-build: docker-prereqs
	docker compose build

docker-up: docker-prereqs
	docker compose up --build -d

docker-down:
	docker compose down

docker-logs:
	docker compose logs -f

clean:
	cargo clean

cloud-deploy:
	@set -eu; \
	project="$(GCP_PROJECT)"; env_name="$(GCP_ENV)"; \
	gcloud config set project "$$project" --quiet; \
	gcloud artifacts repositories describe "$(AR_REPOSITORY)" --location="$(GCP_REGION)" >/dev/null 2>&1 || \
	  gcloud artifacts repositories create "$(AR_REPOSITORY)" --repository-format=docker --location="$(GCP_REGION)" --quiet; \
	gcloud builds submit --tag "$(BACKEND_IMAGE)" --file backend/Dockerfile.cloud . --quiet; \
	gcloud builds submit --tag "$(WEB_IMAGE)" --file web-app/Dockerfile . --quiet; \
	TF_VAR_gcp_project_id="$$project" TF_VAR_gcp_region="$(GCP_REGION)" \
	TF_VAR_backend_image="$(BACKEND_IMAGE)" TF_VAR_web_image="$(WEB_IMAGE)" \
	  terraform -chdir="terraform/environments/$$env_name" init -input=false -backend-config=../../"$$env_name".backend >/dev/null; \
	TF_VAR_gcp_project_id="$$project" TF_VAR_gcp_region="$(GCP_REGION)" \
	TF_VAR_backend_image="$(BACKEND_IMAGE)" TF_VAR_web_image="$(WEB_IMAGE)" \
	  terraform -chdir="terraform/environments/$$env_name" apply -auto-approve -input=false

cloud-deploy-test:
	$(MAKE) cloud-deploy GCP_ENV=test GCP_PROJECT="$(GCP_TEST_PROJECT)"

cloud-deploy-prod:
	$(MAKE) cloud-deploy GCP_ENV=prod GCP_PROJECT="$(GCP_PROD_PROJECT)"

cloud-start:
	@gcloud run services update "robot-fleet-$(GCP_ENV)-backend" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 1 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-web" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 1 --max 1 --quiet

cloud-stop:
	@gcloud run services update "robot-fleet-$(GCP_ENV)-backend" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 0 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-web" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 0 --max 1 --quiet

cloud-start-test:
	$(MAKE) cloud-start GCP_ENV=test GCP_PROJECT="$(GCP_TEST_PROJECT)"

cloud-stop-test:
	$(MAKE) cloud-stop GCP_ENV=test GCP_PROJECT="$(GCP_TEST_PROJECT)"

cloud-start-prod:
	$(MAKE) cloud-start GCP_ENV=prod GCP_PROJECT="$(GCP_PROD_PROJECT)"

cloud-stop-prod:
	$(MAKE) cloud-stop GCP_ENV=prod GCP_PROJECT="$(GCP_PROD_PROJECT)"
