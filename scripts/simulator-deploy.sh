#!/usr/bin/env bash

set -euo pipefail

project="${GCP_PROJECT:?GCP_PROJECT is required}"
env_name="${GCP_ENV:?GCP_ENV is required}"
region="${GCP_REGION:?GCP_REGION is required}"
repository="${AR_REPOSITORY:?AR_REPOSITORY is required}"
simulator_image="${SIMULATOR_IMAGE:?SIMULATOR_IMAGE is required}"
mqtt_url="${SIMULATOR_MQTT_URL:?SIMULATOR_MQTT_URL is required}"
billing_account="${GCP_BILLING_ACCOUNT:-}"

if ! gcloud projects describe "$project" --format='value(projectId)' >/dev/null 2>&1; then
  test -n "$billing_account" || {
    echo "GCP_BILLING_ACCOUNT is required to create project $project" >&2
    exit 1
  }
  gcloud projects create "$project" --name="Robot Fleet Simulators $env_name" --quiet
fi

if [ -n "$billing_account" ]; then
  gcloud billing projects link "$project" --billing-account="$billing_account" --quiet
fi

gcloud auth print-access-token >/dev/null
gcloud services enable \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com \
  run.googleapis.com \
  --project "$project" --quiet

if ! gcloud artifacts repositories describe "$repository" \
  --location="$region" --project="$project" >/dev/null 2>&1; then
  gcloud artifacts repositories create "$repository" \
    --repository-format=docker \
    --location="$region" \
    --project="$project" \
    --quiet
fi

gcloud builds submit . \
  --config=infrastructure/cloud/cloudbuild.yaml \
  --substitutions="_DOCKERFILE=robot-simulator/Dockerfile,_IMAGE=${simulator_image}" \
  --project="$project" --quiet

for robot in \
  "robot-01:Loader One" \
  "robot-02:Picker Two" \
  "robot-03:Scout Three"; do
  robot_id="${robot%%:*}"
  robot_name="${robot#*:}"
  gcloud run deploy "robot-fleet-${env_name}-${robot_id}" \
    --image="$simulator_image" \
    --region="$region" \
    --project="$project" \
    --platform=managed \
    --port=8080 \
    --min=1 \
    --max=1 \
    --cpu=1 \
    --memory=512Mi \
    --no-cpu-throttling \
    --allow-unauthenticated \
    --set-env-vars="ROBOT_ID=${robot_id},ROBOT_NAME=${robot_name},MQTT_URL=${mqtt_url},METRICS_PORT=8080,TELEMETRY_INTERVAL_SECONDS=5,ROBOT_STATE_INTERVAL_SECONDS=1,RUST_LOG=info" \
    --quiet
done
