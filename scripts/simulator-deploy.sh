#!/usr/bin/env bash

set -euo pipefail

project="${GCP_PROJECT:?GCP_PROJECT is required}"
env_name="${GCP_ENV:?GCP_ENV is required}"
region="${GCP_REGION:?GCP_REGION is required}"
repository="${AR_REPOSITORY:?AR_REPOSITORY is required}"
simulator_image="${SIMULATOR_IMAGE:?SIMULATOR_IMAGE is required}"
mqtt_url="${SIMULATOR_MQTT_URL:?SIMULATOR_MQTT_URL is required}"
mqtt_username="${MQTT_USERNAME:?MQTT_USERNAME is required}"
mqtt_password="${MQTT_PASSWORD:?MQTT_PASSWORD is required}"
billing_account="${GCP_BILLING_ACCOUNT:-}"
secret_prefix="robot-fleet-${env_name}"

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

deployer_account="$(gcloud config get-value account 2>/dev/null)"
case "$deployer_account" in
  *@*.gserviceaccount.com) deployer_member="serviceAccount:${deployer_account}" ;;
  *@*) deployer_member="user:${deployer_account}" ;;
  *)
    echo "Unable to determine the active gcloud account" >&2
    exit 1
    ;;
esac

for role in \
  roles/artifactregistry.admin \
  roles/cloudbuild.builds.editor \
  roles/iam.serviceAccountUser \
  roles/run.admin \
  roles/serviceusage.serviceUsageAdmin; do
  gcloud projects add-iam-policy-binding "$project" \
    --member="$deployer_member" \
    --role="$role" \
    --quiet >/dev/null
done

gcloud auth print-access-token >/dev/null
gcloud services enable \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com \
  run.googleapis.com \
  secretmanager.googleapis.com \
  --project "$project" --quiet

simulator_project_number="$(gcloud projects describe "$project" --format='value(projectNumber)')"
simulator_runtime_account="${simulator_project_number}-compute@developer.gserviceaccount.com"
for secret in mqtt-username mqtt-password; do
  gcloud secrets describe "${secret_prefix}-${secret}" --project="$project" >/dev/null 2>&1 ||
    gcloud secrets create "${secret_prefix}-${secret}" \
      --replication-policy=automatic --project="$project" --quiet
done
printf '%s' "$mqtt_username" |
  gcloud secrets versions add "${secret_prefix}-mqtt-username" \
    --data-file=- --project="$project" --quiet
printf '%s' "$mqtt_password" |
  gcloud secrets versions add "${secret_prefix}-mqtt-password" \
    --data-file=- --project="$project" --quiet
for secret in mqtt-username mqtt-password; do
  gcloud secrets add-iam-policy-binding "${secret_prefix}-${secret}" \
    --project="$project" \
    --member="serviceAccount:${simulator_runtime_account}" \
    --role="roles/secretmanager.secretAccessor" \
    --quiet >/dev/null
done

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
    --min-instances=1 \
    --max-instances=1 \
    --cpu=1 \
    --memory=512Mi \
    --no-cpu-throttling \
    --allow-unauthenticated \
    --set-env-vars="ROBOT_ID=${robot_id},ROBOT_NAME=${robot_name},MQTT_URL=${mqtt_url},METRICS_PORT=8080,TELEMETRY_INTERVAL_SECONDS=5,ROBOT_STATE_INTERVAL_SECONDS=1,RUST_LOG=info" \
    --set-secrets="MQTT_USERNAME=${secret_prefix}-mqtt-username:latest,MQTT_PASSWORD=${secret_prefix}-mqtt-password:latest" \
    --quiet
done
