#!/usr/bin/env bash

set -euo pipefail

project="${GCP_PROJECT:?GCP_PROJECT is required}"
env_name="${GCP_ENV:?GCP_ENV is required}"
region="${GCP_REGION:?GCP_REGION is required}"
repository="${AR_REPOSITORY:?AR_REPOSITORY is required}"
backend_image="${BACKEND_IMAGE:?BACKEND_IMAGE is required}"
web_image="${WEB_IMAGE:?WEB_IMAGE is required}"
mqtt_image="${MQTT_IMAGE:?MQTT_IMAGE is required}"
postgres_image="${POSTGRES_IMAGE:?POSTGRES_IMAGE is required}"
use_existing_secrets="${USE_EXISTING_SECRETS:-false}"
image_tag="${IMAGE_TAG:?IMAGE_TAG is required}"
billing_account="${GCP_BILLING_ACCOUNT:-}"
state_bucket="$(awk -F '"' '/^[[:space:]]*bucket[[:space:]]*=/{print $2}' "terraform/${env_name}.backend")"

test -n "$state_bucket"

describe_status=0
describe_output="$(gcloud projects describe "$project" --format='value(projectId)' 2>&1)" || describe_status=$?
if [ "$describe_status" -ne 0 ]; then
  printf '%s\n' "$describe_output" >&2
  resource_manager_project="$(
    printf '%s\n' "$describe_output" |
      sed -n 's/.*project \([0-9][0-9]*\) before or it is disabled.*/\1/p' |
      head -n 1
  )"
  if printf '%s\n' "$describe_output" |
    grep -Eq 'reason: SERVICE_DISABLED|has not been used in project .* before or it is disabled' &&
    [ -n "$resource_manager_project" ]; then
    echo "Enabling Cloud Resource Manager API in project $resource_manager_project..." >&2
    gcloud services enable cloudresourcemanager.googleapis.com \
      --project="$resource_manager_project" --quiet
    sleep 5
  else
    exit "$describe_status"
  fi
fi

if ! gcloud projects describe "$project" --format='value(projectId)' >/dev/null 2>&1; then
  test -n "$billing_account" || {
    echo "GCP_BILLING_ACCOUNT is required to create project $project" >&2
    exit 1
  }
  gcloud projects create "$project" --name="Robot Fleet $env_name" --quiet
fi

if [ -n "$billing_account" ]; then
  current_billing_account="$(
    gcloud billing projects describe "$project" \
      --format='value(billingAccountName)' 2>/dev/null || true
  )"
  desired_billing_account="billingAccounts/${billing_account}"
  if [ "$current_billing_account" != "$desired_billing_account" ]; then
    gcloud billing projects link "$project" --billing-account="$billing_account" --quiet
  fi
fi

gcloud config set project "$project" --quiet
gcloud auth print-access-token >/dev/null

if [ -z "${GOOGLE_GHA_CREDS_PATH:-}" ]; then
  gcloud auth application-default print-access-token >/dev/null
  gcloud auth application-default set-quota-project "$project" --quiet
fi

gcloud services enable \
  cloudbuild.googleapis.com \
  artifactregistry.googleapis.com \
  secretmanager.googleapis.com \
  run.googleapis.com \
  vpcaccess.googleapis.com \
  --project "$project" --quiet

if [ "$use_existing_secrets" = true ]; then
  secret_prefix="robot-fleet-${env_name}"
  postgres_username="$(gcloud secrets versions access latest --secret="${secret_prefix}-postgres-username" --project="$project")"
  postgres_password="$(gcloud secrets versions access latest --secret="${secret_prefix}-postgres-password" --project="$project")"
  mqtt_username="$(gcloud secrets versions access latest --secret="${secret_prefix}-mqtt-username" --project="$project")"
  mqtt_password="$(gcloud secrets versions access latest --secret="${secret_prefix}-mqtt-password" --project="$project")"
else
  postgres_username="${POSTGRES_USERNAME:?POSTGRES_USERNAME is required}"
  postgres_password="${POSTGRES_PASSWORD:?POSTGRES_PASSWORD is required}"
  mqtt_username="${MQTT_USERNAME:?MQTT_USERNAME is required}"
  mqtt_password="${MQTT_PASSWORD:?MQTT_PASSWORD is required}"
fi

gcloud storage buckets describe "gs://${state_bucket}" --project "$project" >/dev/null 2>&1 ||
  gcloud storage buckets create "gs://${state_bucket}" \
    --project "$project" \
    --location="$region" \
    --uniform-bucket-level-access

if ! gcloud artifacts repositories describe "$repository" \
  --location="$region" --project="$project" >/dev/null 2>&1; then
  created=0
  for attempt in 1 2 3 4 5; do
    if gcloud artifacts repositories create "$repository" \
      --repository-format=docker \
      --location="$region" \
      --project="$project" \
      --quiet; then
      created=1
      break
    fi
    if [ "$attempt" -lt 5 ]; then
      echo "Artifact Registry is not ready yet; retrying ($attempt/5)..." >&2
      sleep 10
    fi
  done
  test "$created" -eq 1
fi

gcloud builds submit . \
  --config=infrastructure/cloud/cloudbuild.yaml \
  --substitutions="_DOCKERFILE=backend/Dockerfile.cloud,_IMAGE=${backend_image}" \
  --project="$project" --quiet --suppress-logs
gcloud builds submit . \
  --config=infrastructure/cloud/cloudbuild.yaml \
  --substitutions="_DOCKERFILE=web-app/Dockerfile,_IMAGE=${web_image}" \
  --project="$project" --quiet --suppress-logs
gcloud builds submit . \
  --config=infrastructure/cloud/cloudbuild.yaml \
  --substitutions="_DOCKERFILE=infrastructure/mqtt/Dockerfile.cloud,_IMAGE=${mqtt_image}" \
  --project="$project" --quiet --suppress-logs
gcloud builds submit . \
  --config=infrastructure/cloud/cloudbuild.yaml \
  --substitutions="_DOCKERFILE=infrastructure/postgres/Dockerfile,_IMAGE=${postgres_image}" \
  --project="$project" --quiet --suppress-logs

terraform_env=(
  "TF_VAR_gcp_project_id=$project"
  "TF_VAR_gcp_region=$region"
  "TF_VAR_backend_image=$backend_image"
  "TF_VAR_web_image=$web_image"
  "TF_VAR_mqtt_image=$mqtt_image"
  "TF_VAR_postgres_image=$postgres_image"
  "TF_VAR_postgres_username=$postgres_username"
  "TF_VAR_postgres_password=$postgres_password"
  "TF_VAR_mqtt_username=$mqtt_username"
  "TF_VAR_mqtt_password=$mqtt_password"
  "TF_VAR_image_tag=$image_tag"
)

env "${terraform_env[@]}" terraform \
  -chdir="terraform/environments/${env_name}" \
  init -input=false -backend-config="../../${env_name}.backend" >/dev/null

if gcloud artifacts repositories describe "$repository" \
  --location="$region" --project="$project" >/dev/null 2>&1; then
  env "${terraform_env[@]}" terraform \
    -chdir="terraform/environments/${env_name}" \
    state show 'module.environment.module.google[0].google_artifact_registry_repository.images' \
    >/dev/null 2>&1 ||
    env "${terraform_env[@]}" terraform \
      -chdir="terraform/environments/${env_name}" \
      import -input=false \
      'module.environment.module.google[0].google_artifact_registry_repository.images' \
      "projects/${project}/locations/${region}/repositories/${repository}" >/dev/null
fi

env "${terraform_env[@]}" terraform \
  -chdir="terraform/environments/${env_name}" \
  apply -auto-approve -input=false
