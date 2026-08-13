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
GCP_BILLING_ACCOUNT ?=
REPO ?= satr/robot-fleet
AR_REPOSITORY ?= robot-fleet-$(GCP_ENV)-images
AR_HOST ?= $(GCP_REGION)-docker.pkg.dev
IMAGE_TAG ?= run-$(shell date -u +%Y%m%d%H%M%S)-$(shell uuidgen | cut -d- -f1)
BACKEND_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/backend:$(IMAGE_TAG)
WEB_IMAGE ?= $(AR_HOST)/$(GCP_PROJECT)/$(AR_REPOSITORY)/web:$(IMAGE_TAG)

.PHONY: help infra-up infra-down infra-logs db-migrate backend-run backend-stop-dev backend-test web-run web-stop-dev web-build web-check robot-run robot-dev robot1-run robot2-run robot3-run robots-run robot1-dev robot2-dev robot3-dev robots-dev robots-stop-dev stop-dev robot1-up robot2-up robot3-up robots-up robot1-down robot2-down robot3-down robots-down dev build test docker-prereqs docker-build docker-up docker-down docker-logs cloud-deploy cloud-deploy-test cloud-deploy-prod cloud-github-auth cloud-github-auth-test cloud-github-auth-prod cloud-start cloud-stop cloud-start-test cloud-stop-test cloud-start-prod cloud-stop-prod clean

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
	@echo "  make cloud-github-auth-test  Create test GitHub OIDC auth and set repository secrets"
	@echo "  make cloud-github-auth-prod  Create prod GitHub OIDC auth and set repository secrets"
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
	billing_account="$(GCP_BILLING_ACCOUNT)"; \
	state_bucket="$$(awk -F '"' '/^[[:space:]]*bucket[[:space:]]*=/{print $$2}' "terraform/$$env_name.backend")"; \
	test -n "$$state_bucket"; \
	if ! gcloud projects describe "$$project" --format='value(projectId)' >/dev/null 2>&1; then \
	  test -n "$$billing_account" || { \
	    echo "GCP_BILLING_ACCOUNT is required to create project $$project" >&2; \
	    exit 1; \
	  }; \
	  gcloud projects create "$$project" --name="Robot Fleet $$env_name" --quiet; \
	fi; \
	if [ -n "$$billing_account" ]; then \
	  gcloud billing projects link "$$project" --billing-account="$$billing_account" --quiet; \
	fi; \
	gcloud config set project "$$project" --quiet; \
	gcloud auth print-access-token >/dev/null; \
	if [ -n "$${GOOGLE_GHA_CREDS_PATH:-}" ]; then \
	  :; \
	else \
	  gcloud auth application-default print-access-token >/dev/null; \
	  gcloud auth application-default set-quota-project "$$project" --quiet; \
	fi; \
	gcloud services enable cloudbuild.googleapis.com artifactregistry.googleapis.com run.googleapis.com --project "$$project" --quiet; \
	gcloud storage buckets describe "gs://$$state_bucket" --project "$$project" >/dev/null 2>&1 || \
	  gcloud storage buckets create "gs://$$state_bucket" --project "$$project" --location="$(GCP_REGION)" --uniform-bucket-level-access; \
	if ! gcloud artifacts repositories describe "$(AR_REPOSITORY)" --location="$(GCP_REGION)" --project="$$project" >/dev/null 2>&1; then \
	  created=0; \
	  for attempt in 1 2 3 4 5; do \
	    if gcloud artifacts repositories create "$(AR_REPOSITORY)" --repository-format=docker --location="$(GCP_REGION)" --project="$$project" --quiet; then \
	      created=1; \
	      break; \
	    fi; \
	    if [ "$$attempt" -lt 5 ]; then \
	      echo "Artifact Registry is not ready yet; retrying ($$attempt/5)..." >&2; \
	      sleep 10; \
	    fi; \
	  done; \
	  test "$$created" -eq 1; \
	fi; \
	gcloud builds submit . --config=infrastructure/cloud/cloudbuild.yaml \
	  --substitutions="_DOCKERFILE=backend/Dockerfile.cloud,_IMAGE=$(BACKEND_IMAGE)" \
	  --project="$$project" --quiet; \
	gcloud builds submit . --config=infrastructure/cloud/cloudbuild.yaml \
	  --substitutions="_DOCKERFILE=web-app/Dockerfile,_IMAGE=$(WEB_IMAGE)" \
	  --project="$$project" --quiet; \
	TF_VAR_gcp_project_id="$$project" TF_VAR_gcp_region="$(GCP_REGION)" \
	TF_VAR_backend_image="$(BACKEND_IMAGE)" TF_VAR_web_image="$(WEB_IMAGE)" TF_VAR_image_tag="$(IMAGE_TAG)" \
	  terraform -chdir="terraform/environments/$$env_name" init -input=false -backend-config=../../"$$env_name".backend >/dev/null; \
	if gcloud artifacts repositories describe "$(AR_REPOSITORY)" --location="$(GCP_REGION)" --project="$$project" >/dev/null 2>&1; then \
	  TF_VAR_gcp_project_id="$$project" TF_VAR_gcp_region="$(GCP_REGION)" \
	  terraform -chdir="terraform/environments/$$env_name" state show 'module.environment.module.google[0].google_artifact_registry_repository.images' >/dev/null 2>&1 || \
	  terraform -chdir="terraform/environments/$$env_name" import -input=false \
	    'module.environment.module.google[0].google_artifact_registry_repository.images' \
	    "projects/$$project/locations/$(GCP_REGION)/repositories/$(AR_REPOSITORY)" >/dev/null; \
	fi; \
	TF_VAR_gcp_project_id="$$project" TF_VAR_gcp_region="$(GCP_REGION)" \
	TF_VAR_backend_image="$(BACKEND_IMAGE)" TF_VAR_web_image="$(WEB_IMAGE)" TF_VAR_image_tag="$(IMAGE_TAG)" \
	  terraform -chdir="terraform/environments/$$env_name" apply -auto-approve -input=false

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

cloud-github-auth:
	@set -eu; \
	project="$${GCP_PROJECT_ID:-$(GCP_PROJECT)}"; \
	repo="$${REPO:-$(REPO)}"; \
	pool_id="$${GCP_WIF_POOL_ID:-github}"; \
	provider_id="$${GCP_WIF_PROVIDER_ID:-github}"; \
	service_account_id="$${GCP_DEPLOY_SERVICE_ACCOUNT_ID:-github-deployer}"; \
	test -n "$$project" || { echo "GCP_PROJECT_ID is required" >&2; exit 1; }; \
	test -n "$$repo" || { echo "REPO is required" >&2; exit 1; }; \
	project_number="$$(gcloud projects describe "$$project" --format='value(projectNumber)')"; \
	service_account="$$service_account_id@$$project.iam.gserviceaccount.com"; \
	gcloud services enable iam.googleapis.com iamcredentials.googleapis.com sts.googleapis.com --project="$$project" --quiet; \
	if ! gcloud iam workload-identity-pools describe "$$pool_id" --project="$$project" --location=global >/dev/null 2>&1; then \
	  gcloud iam workload-identity-pools create "$$pool_id" --project="$$project" --location=global --display-name="GitHub Actions" --quiet; \
	fi; \
	if ! gcloud iam workload-identity-pools providers describe "$$provider_id" --project="$$project" --location=global --workload-identity-pool="$$pool_id" >/dev/null 2>&1; then \
	  gcloud iam workload-identity-pools providers create-oidc "$$provider_id" \
	    --project="$$project" --location=global --workload-identity-pool="$$pool_id" \
	    --display-name="GitHub" --issuer-uri="https://token.actions.githubusercontent.com/" \
	    --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.repository_owner=assertion.repository_owner" \
	    --attribute-condition="assertion.repository == '$$repo'" --quiet; \
	fi; \
	if ! gcloud iam service-accounts describe "$$service_account" --project="$$project" >/dev/null 2>&1; then \
	  gcloud iam service-accounts create "$$service_account_id" --project="$$project" --display-name="GitHub Actions deployer" --quiet; \
	fi; \
	gcloud iam service-accounts add-iam-policy-binding "$$service_account" --project="$$project" \
	  --role=roles/iam.workloadIdentityUser \
	  --member="principalSet://iam.googleapis.com/projects/$$project_number/locations/global/workloadIdentityPools/$$pool_id/attribute.repository/$$repo" --quiet; \
	for role in roles/cloudbuild.builds.editor roles/artifactregistry.admin roles/run.admin roles/storage.admin roles/serviceusage.serviceUsageAdmin roles/iam.serviceAccountUser; do \
	  gcloud projects add-iam-policy-binding "$$project" --member="serviceAccount:$$service_account" --role="$$role" --quiet >/dev/null; \
	done; \
	provider="projects/$$project_number/locations/global/workloadIdentityPools/$$pool_id/providers/$$provider_id"; \
	printf '%s' "$$provider" | gh secret set GCP_WORKLOAD_IDENTITY_PROVIDER --repo "$$repo"; \
	printf '%s' "$$service_account" | gh secret set GCP_DEPLOY_SERVICE_ACCOUNT --repo "$$repo"; \
	echo "Configured GitHub OIDC auth for $$project in $$repo"

cloud-github-auth-test:
	@set -a; [ ! -f .env.test ] || . ./.env.test; set +a; \
	$(MAKE) cloud-github-auth GCP_ENV=test GCP_PROJECT_ID="$${GCP_PROJECT_ID:-$(GCP_TEST_PROJECT)}"

cloud-github-auth-prod:
	@set -a; [ ! -f .env.prod ] || . ./.env.prod; set +a; \
	$(MAKE) cloud-github-auth GCP_ENV=prod GCP_PROJECT_ID="$${GCP_PROJECT_ID:-$(GCP_PROD_PROJECT)}"

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
