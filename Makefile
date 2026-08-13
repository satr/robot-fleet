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
GCP_TEST_PROJECT ?= robot-fleet-test-00000001
GCP_PROD_PROJECT ?= robot-fleet-prod-00000001
GCP_SIMULATOR_TEST_PROJECT ?= robot-fleet-sim-test-00000001
GCP_SIMULATOR_PROD_PROJECT ?= robot-fleet-sim-prod-00000001
GCP_ENV ?= test
GCP_PROJECT ?= $(if $(filter prod,$(GCP_ENV)),$(GCP_PROD_PROJECT),$(GCP_TEST_PROJECT))
GCP_SIMULATOR_PROJECT ?= $(if $(filter prod,$(GCP_ENV)),$(GCP_SIMULATOR_PROD_PROJECT),$(GCP_SIMULATOR_TEST_PROJECT))
GCP_BILLING_ACCOUNT ?=
SIMULATOR_MQTT_URL ?=
REPO ?= satr/robot-fleet
AR_REPOSITORY ?= robot-fleet-$(GCP_ENV)-images
SIMULATOR_AR_REPOSITORY ?= robot-fleet-$(GCP_ENV)-simulator-images
AR_HOST ?= $(GCP_REGION)-docker.pkg.dev
IMAGE_TAG ?= run-$(shell date -u +%Y%m%d%H%M%S)-$(shell uuidgen | cut -d- -f1)
BACKEND_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/backend:$(IMAGE_TAG)
WEB_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/web:$(IMAGE_TAG)
MQTT_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/mqtt:$(IMAGE_TAG)
POSTGRES_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/postgres:$(IMAGE_TAG)
SIMULATOR_IMAGE ?= $(AR_HOST)/$(GCP_SIMULATOR_PROJECT)/$(SIMULATOR_AR_REPOSITORY)/simulator:$(IMAGE_TAG)

.PHONY: help infra-up infra-down infra-logs db-migrate backend-run backend-stop-dev backend-test web-run web-stop-dev web-build web-check robot-run robot-dev robot1-run robot2-run robot3-run robots-run robot1-dev robot2-dev robot3-dev robots-dev robots-stop-dev stop-dev robot1-up robot2-up robot3-up robots-up robot1-down robot2-down robot3-down robots-down dev build test docker-prereqs docker-build docker-up docker-down docker-logs cloud-deploy cloud-deploy-test cloud-deploy-prod cloud-github-auth cloud-github-auth-test cloud-github-auth-prod cloud-start cloud-stop cloud-start-test cloud-stop-test cloud-start-prod cloud-stop-prod simulator-deploy simulator-deploy-test simulator-deploy-prod simulator-start simulator-stop simulator-start-test simulator-stop-test simulator-start-prod simulator-stop-prod simulator-github-auth simulator-github-auth-test simulator-github-auth-prod clean
.PHONY: cloud-secrets cloud-secrets-test cloud-secrets-prod

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
	@echo "  make cloud-secrets-test  Refresh test PostgreSQL and MQTT secrets from .env.test"
	@echo "  make cloud-secrets-prod  Refresh prod PostgreSQL and MQTT secrets from .env.prod"
	@echo "  make cloud-github-auth-test  Create test GitHub OIDC auth and set repository secrets"
	@echo "  make cloud-github-auth-prod  Create prod GitHub OIDC auth and set repository secrets"
	@echo "  make cloud-start GCP_ENV=test|prod  Restore one Cloud Run instance"
	@echo "  make cloud-stop GCP_ENV=test|prod   Scale Cloud Run services to zero"
	@echo "  make simulator-deploy-test  Deploy test robot simulators to GCP_SIMULATOR_TEST_PROJECT"
	@echo "  make simulator-deploy-prod  Deploy prod robot simulators to GCP_SIMULATOR_PROD_PROJECT"
	@echo "  make simulator-start GCP_ENV=test|prod  Restore simulator instances"
	@echo "  make simulator-stop GCP_ENV=test|prod   Scale simulator services to zero"
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
	docker pull timescale/timescaledb:latest-pg16
	docker pull eclipse-mosquitto:2
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
	@GCP_PROJECT="$(GCP_PROJECT)" \
	GCP_ENV="$(GCP_ENV)" \
	GCP_REGION="$(GCP_REGION)" \
	GCP_BILLING_ACCOUNT="$(GCP_BILLING_ACCOUNT)" \
	AR_REPOSITORY="$(AR_REPOSITORY)" \
	BACKEND_IMAGE="$(BACKEND_IMAGE)" \
	WEB_IMAGE="$(WEB_IMAGE)" \
	MQTT_IMAGE="$(MQTT_IMAGE)" \
	POSTGRES_IMAGE="$(POSTGRES_IMAGE)" \
	USE_EXISTING_SECRETS="$(USE_EXISTING_SECRETS)" \
	IMAGE_TAG="$(IMAGE_TAG)" \
		scripts/cloud-deploy.sh

cloud-deploy-test:
	@set -eu; \
	if [ -f .env.test ]; then set -a; . ./.env.test; set +a; fi; \
	project="$${GCP_PROJECT_ID:-$(GCP_TEST_PROJECT)}"; \
	tag="$${IMAGE_TAG:-$(IMAGE_TAG)}"; \
	$(MAKE) cloud-deploy GCP_ENV=test GCP_PROJECT="$$project" IMAGE_TAG="$$tag" TF_VAR_image_tag="$$tag"

cloud-deploy-prod:
	@set -eu; \
	if [ -f .env.prod ]; then set -a; . ./.env.prod; set +a; fi; \
	project="$${GCP_PROJECT_ID:-$(GCP_PROD_PROJECT)}"; \
	tag="$${IMAGE_TAG:-$(IMAGE_TAG)}"; \
	$(MAKE) cloud-deploy GCP_ENV=prod GCP_PROJECT="$$project" IMAGE_TAG="$$tag" TF_VAR_image_tag="$$tag"

cloud-secrets:
	@set -eu; \
	project="$(GCP_PROJECT)"; \
	prefix="robot-fleet-$(GCP_ENV)"; \
	test -n "$${POSTGRES_USERNAME:-}" || { echo "POSTGRES_USERNAME is required" >&2; exit 1; }; \
	test -n "$${POSTGRES_PASSWORD:-}" || { echo "POSTGRES_PASSWORD is required" >&2; exit 1; }; \
	test -n "$${MQTT_USERNAME:-}" || { echo "MQTT_USERNAME is required" >&2; exit 1; }; \
	test -n "$${MQTT_PASSWORD:-}" || { echo "MQTT_PASSWORD is required" >&2; exit 1; }; \
	for secret in postgres-username postgres-password mqtt-username mqtt-password; do \
		if ! gcloud secrets describe "$$prefix-$$secret" --project="$$project" >/dev/null 2>&1; then \
			gcloud secrets create "$$prefix-$$secret" --replication-policy=automatic --project="$$project" --quiet; \
		fi; \
	done; \
	printf '%s' "$$POSTGRES_USERNAME" | gcloud secrets versions add "$$prefix-postgres-username" --project="$$project" --data-file=- --quiet; \
	printf '%s' "$$POSTGRES_PASSWORD" | gcloud secrets versions add "$$prefix-postgres-password" --project="$$project" --data-file=- --quiet; \
	printf '%s' "$$MQTT_USERNAME" | gcloud secrets versions add "$$prefix-mqtt-username" --project="$$project" --data-file=- --quiet; \
	printf '%s' "$$MQTT_PASSWORD" | gcloud secrets versions add "$$prefix-mqtt-password" --project="$$project" --data-file=- --quiet

cloud-secrets-test:
	@set -a; [ ! -f .env.test ] || . ./.env.test; set +a; \
	$(MAKE) cloud-secrets GCP_ENV=test GCP_PROJECT="$${GCP_PROJECT_ID:-$(GCP_TEST_PROJECT)}"

cloud-secrets-prod:
	@set -a; [ ! -f .env.prod ] || . ./.env.prod; set +a; \
	$(MAKE) cloud-secrets GCP_ENV=prod GCP_PROJECT="$${GCP_PROJECT_ID:-$(GCP_PROD_PROJECT)}"

cloud-github-auth:
	@GCP_PROJECT_ID="$${GCP_PROJECT_ID:-$(GCP_PROJECT)}" \
	REPO="$${REPO:-$(REPO)}" \
	GCP_WIF_POOL_ID="$${GCP_WIF_POOL_ID:-github}" \
	GCP_WIF_PROVIDER_ID="$${GCP_WIF_PROVIDER_ID:-github}" \
	GCP_DEPLOY_SERVICE_ACCOUNT_ID="$${GCP_DEPLOY_SERVICE_ACCOUNT_ID:-github-deployer}" \
		scripts/cloud-github-auth.sh

cloud-github-auth-test:
	@set -a; [ ! -f .env.test ] || . ./.env.test; set +a; \
	$(MAKE) cloud-github-auth GCP_ENV=test GCP_PROJECT_ID="$${GCP_PROJECT_ID:-$(GCP_TEST_PROJECT)}"

cloud-github-auth-prod:
	@set -a; [ ! -f .env.prod ] || . ./.env.prod; set +a; \
	$(MAKE) cloud-github-auth GCP_ENV=prod GCP_PROJECT_ID="$${GCP_PROJECT_ID:-$(GCP_PROD_PROJECT)}"

cloud-start:
	@gcloud run services update "robot-fleet-$(GCP_ENV)-backend" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 1 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-mqtt" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 1 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-web" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 1 --max 1 --quiet

cloud-stop:
	@gcloud run services update "robot-fleet-$(GCP_ENV)-backend" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 0 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-mqtt" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 0 --max 1 --quiet
	@gcloud run services update "robot-fleet-$(GCP_ENV)-web" --region "$(GCP_REGION)" --project "$(GCP_PROJECT)" --min 0 --max 1 --quiet

cloud-start-test:
	$(MAKE) cloud-start GCP_ENV=test GCP_PROJECT="$(GCP_TEST_PROJECT)"

cloud-stop-test:
	$(MAKE) cloud-stop GCP_ENV=test GCP_PROJECT="$(GCP_TEST_PROJECT)"

cloud-start-prod:
	$(MAKE) cloud-start GCP_ENV=prod GCP_PROJECT="$(GCP_PROD_PROJECT)"

cloud-stop-prod:
	$(MAKE) cloud-stop GCP_ENV=prod GCP_PROJECT="$(GCP_PROD_PROJECT)"

simulator-deploy:
	@GCP_PROJECT="$(GCP_SIMULATOR_PROJECT)" \
	GCP_ENV="$(GCP_ENV)" \
	GCP_REGION="$(GCP_REGION)" \
	GCP_BILLING_ACCOUNT="$(GCP_BILLING_ACCOUNT)" \
	AR_REPOSITORY="$(SIMULATOR_AR_REPOSITORY)" \
	SIMULATOR_IMAGE="$(SIMULATOR_IMAGE)" \
	SIMULATOR_MQTT_URL="$(SIMULATOR_MQTT_URL)" \
	MQTT_USERNAME="$(MQTT_USERNAME)" \
	MQTT_PASSWORD="$(MQTT_PASSWORD)" \
	SIMULATOR_USE_EXISTING_SECRETS="$(SIMULATOR_USE_EXISTING_SECRETS)" \
		./scripts/simulator-deploy.sh

simulator-deploy-test:
	@set -eu; \
	if [ -f .env.test ]; then set -a; . ./.env.test; set +a; fi; \
	project="$${GCP_SIMULATOR_PROJECT_ID:-$(GCP_SIMULATOR_TEST_PROJECT)}"; \
	mqtt_url="$${SIMULATOR_MQTT_URL:-}"; \
	test -n "$$mqtt_url" || { echo "SIMULATOR_MQTT_URL is required in .env.test or the environment" >&2; exit 1; }; \
	$(MAKE) simulator-deploy GCP_ENV=test GCP_PROJECT="$$project" GCP_SIMULATOR_PROJECT="$$project" SIMULATOR_MQTT_URL="$$mqtt_url"

simulator-deploy-prod:
	@set -eu; \
	if [ -f .env.prod ]; then set -a; . ./.env.prod; set +a; fi; \
	project="$${GCP_SIMULATOR_PROJECT_ID:-$(GCP_SIMULATOR_PROD_PROJECT)}"; \
	mqtt_url="$${SIMULATOR_MQTT_URL:-}"; \
	test -n "$$mqtt_url" || { echo "SIMULATOR_MQTT_URL is required in .env.prod or the environment" >&2; exit 1; }; \
	$(MAKE) simulator-deploy GCP_ENV=prod GCP_PROJECT="$$project" GCP_SIMULATOR_PROJECT="$$project" SIMULATOR_MQTT_URL="$$mqtt_url"

simulator-start:
	@for robot_id in robot-01 robot-02 robot-03; do \
		gcloud run services update "robot-fleet-$(GCP_ENV)-$$robot_id" --region "$(GCP_REGION)" --project "$(GCP_SIMULATOR_PROJECT)" --min 1 --max 1 --quiet; \
	done

simulator-stop:
	@for robot_id in robot-01 robot-02 robot-03; do \
		gcloud run services update "robot-fleet-$(GCP_ENV)-$$robot_id" --region "$(GCP_REGION)" --project "$(GCP_SIMULATOR_PROJECT)" --min 0 --max 1 --quiet; \
	done

simulator-start-test:
	$(MAKE) simulator-start GCP_ENV=test GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_TEST_PROJECT)"

simulator-stop-test:
	$(MAKE) simulator-stop GCP_ENV=test GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_TEST_PROJECT)"

simulator-start-prod:
	$(MAKE) simulator-start GCP_ENV=prod GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_PROD_PROJECT)"

simulator-stop-prod:
	$(MAKE) simulator-stop GCP_ENV=prod GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_PROD_PROJECT)"

simulator-github-auth:
	@GCP_PROJECT_ID="$${GCP_SIMULATOR_PROJECT_ID:-$(GCP_SIMULATOR_PROJECT)}" \
	SIMULATOR_MQTT_URL="$${SIMULATOR_MQTT_URL:-$(SIMULATOR_MQTT_URL)}" \
	REPO="$${REPO:-$(REPO)}" \
	GCP_WIF_POOL_ID="$${GCP_SIMULATOR_WIF_POOL_ID:-github}" \
	GCP_WIF_PROVIDER_ID="$${GCP_SIMULATOR_WIF_PROVIDER_ID:-github}" \
	GCP_DEPLOY_SERVICE_ACCOUNT_ID="$${GCP_SIMULATOR_DEPLOY_SERVICE_ACCOUNT_ID:-github-simulator-deployer}" \
	GITHUB_WIF_PROVIDER_SECRET=GCP_SIMULATOR_WORKLOAD_IDENTITY_PROVIDER \
	GITHUB_DEPLOY_SERVICE_ACCOUNT_SECRET=GCP_SIMULATOR_DEPLOY_SERVICE_ACCOUNT \
	GITHUB_PROJECT_VARIABLE="$(GITHUB_PROJECT_VARIABLE)" \
	GITHUB_MQTT_URL_VARIABLE="$(GITHUB_MQTT_URL_VARIABLE)" \
		scripts/cloud-github-auth.sh

simulator-github-auth-test:
	@set -a; [ ! -f .env.test ] || . ./.env.test; set +a; \
	mqtt_url="$${SIMULATOR_MQTT_URL:-}"; \
	if [ -z "$$mqtt_url" ]; then \
		mqtt_url="$$(gcloud run services describe robot-fleet-test-mqtt --region "$(GCP_REGION)" --project "$${GCP_PROJECT_ID}" --format='value(status.url)' 2>/dev/null | sed 's#^https://#wss://#')"; \
	fi; \
	test -n "$$mqtt_url" || { echo "SIMULATOR_MQTT_URL is required in .env.test or from robot-fleet-test-mqtt" >&2; exit 1; }; \
	$(MAKE) simulator-github-auth GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_TEST_PROJECT)" SIMULATOR_MQTT_URL="$$mqtt_url" GITHUB_PROJECT_VARIABLE=GCP_SIMULATOR_TEST_PROJECT GITHUB_MQTT_URL_VARIABLE=SIMULATOR_MQTT_URL_TEST

simulator-github-auth-prod:
	@set -a; [ ! -f .env.prod ] || . ./.env.prod; set +a; \
	mqtt_url="$${SIMULATOR_MQTT_URL:-}"; \
	if [ -z "$$mqtt_url" ]; then \
		mqtt_url="$$(gcloud run services describe robot-fleet-prod-mqtt --region "$(GCP_REGION)" --project "$${GCP_PROJECT_ID}" --format='value(status.url)' 2>/dev/null | sed 's#^https://#wss://#')"; \
	fi; \
	test -n "$$mqtt_url" || { echo "SIMULATOR_MQTT_URL is required in .env.prod or from robot-fleet-prod-mqtt" >&2; exit 1; }; \
	$(MAKE) simulator-github-auth GCP_SIMULATOR_PROJECT="$(GCP_SIMULATOR_PROD_PROJECT)" SIMULATOR_MQTT_URL="$$mqtt_url" GITHUB_PROJECT_VARIABLE=GCP_SIMULATOR_PROD_PROJECT GITHUB_MQTT_URL_VARIABLE=SIMULATOR_MQTT_URL_PROD
