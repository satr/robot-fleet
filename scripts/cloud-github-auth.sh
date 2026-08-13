#!/usr/bin/env bash

set -euo pipefail

project="${GCP_PROJECT_ID:?GCP_PROJECT_ID is required}"
repo="${REPO:?REPO is required}"
pool_id="${GCP_WIF_POOL_ID:-github}"
provider_id="${GCP_WIF_PROVIDER_ID:-github}"
service_account_id="${GCP_DEPLOY_SERVICE_ACCOUNT_ID:-github-deployer}"
provider_secret="${GITHUB_WIF_PROVIDER_SECRET:-GCP_WORKLOAD_IDENTITY_PROVIDER}"
service_account_secret="${GITHUB_DEPLOY_SERVICE_ACCOUNT_SECRET:-GCP_DEPLOY_SERVICE_ACCOUNT}"
project_variable="${GITHUB_PROJECT_VARIABLE:-}"

gcloud services enable \
  cloudresourcemanager.googleapis.com \
  iam.googleapis.com \
  iamcredentials.googleapis.com \
  sts.googleapis.com \
  --project="$project" --quiet

project_number="$(gcloud projects describe "$project" --format='value(projectNumber)')"
service_account="${service_account_id}@${project}.iam.gserviceaccount.com"

if ! gcloud iam workload-identity-pools describe "$pool_id" \
  --project="$project" --location=global >/dev/null 2>&1; then
  gcloud iam workload-identity-pools create "$pool_id" \
    --project="$project" \
    --location=global \
    --display-name="GitHub Actions" \
    --quiet
fi

if ! gcloud iam workload-identity-pools providers describe "$provider_id" \
  --project="$project" \
  --location=global \
  --workload-identity-pool="$pool_id" >/dev/null 2>&1; then
  gcloud iam workload-identity-pools providers create-oidc "$provider_id" \
    --project="$project" \
    --location=global \
    --workload-identity-pool="$pool_id" \
    --display-name="GitHub" \
    --issuer-uri="https://token.actions.githubusercontent.com/" \
    --attribute-mapping="google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.repository_owner=assertion.repository_owner" \
    --attribute-condition="assertion.repository == '$repo'" \
    --quiet
fi

if ! gcloud iam service-accounts describe "$service_account" \
  --project="$project" >/dev/null 2>&1; then
  gcloud iam service-accounts create "$service_account_id" \
    --project="$project" \
    --display-name="GitHub Actions deployer" \
    --quiet
fi

gcloud iam service-accounts add-iam-policy-binding "$service_account" \
  --project="$project" \
  --role=roles/iam.workloadIdentityUser \
  --member="principalSet://iam.googleapis.com/projects/${project_number}/locations/global/workloadIdentityPools/${pool_id}/attribute.repository/${repo}" \
  --quiet

for role in \
  roles/cloudbuild.builds.editor \
  roles/artifactregistry.admin \
  roles/run.admin \
  roles/resourcemanager.projectIamAdmin \
  roles/secretmanager.admin \
  roles/storage.admin \
  roles/serviceusage.serviceUsageAdmin \
  roles/iam.serviceAccountUser; do
  gcloud projects add-iam-policy-binding "$project" \
    --member="serviceAccount:${service_account}" \
    --role="$role" \
    --quiet >/dev/null
done

provider="projects/${project_number}/locations/global/workloadIdentityPools/${pool_id}/providers/${provider_id}"
set_github_secret() {
  local secret_name="$1"
  local secret_value="$2"
  local attempt

  for attempt in 1 2 3 4 5; do
    if printf '%s' "$secret_value" | gh secret set "$secret_name" --repo "$repo"; then
      return 0
    fi
    if [ "$attempt" -lt 5 ]; then
      echo "GitHub secret update failed; retrying ($attempt/5)..." >&2
      sleep $((attempt * 2))
    fi
  done

  echo "Failed to set GitHub secret '$secret_name' after 5 attempts" >&2
  return 1
}

set_github_secret "$provider_secret" "$provider"
set_github_secret "$service_account_secret" "$service_account"
if [ -n "$project_variable" ]; then
  gh variable set "$project_variable" --repo "$repo" --body "$project"
fi
echo "Configured GitHub OIDC auth for $project in $repo"
