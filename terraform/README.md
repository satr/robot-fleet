# Robot Fleet Terraform

This configuration deploys the current platform to Google Cloud:

- Cloud Run: backend, SvelteKit web app, and three robot simulators
- Cloud SQL for PostgreSQL 16 with private networking
- Compute Engine: MQTT broker
- Artifact Registry: container images
- VPC, Serverless VPC Access, firewall, backups, and Cloud Run IAM

The prototype currently has no authentication or TLS. MQTT port 1883 is therefore intentionally public to support the simulators; secure this before production use.

## Setup

Create an existing Google Cloud project with a unique ID such as `robot-fleet-alice`, enable billing, and authenticate with Application Default Credentials. Terraform settings are kept in environment files rather than committed tfvars files:

```sh
gcloud auth application-default login
cp .env.test.example .env.test
${EDITOR:-vi} .env.test
```

Build and push the three images to the Artifact Registry repository created by Terraform, then update the project, region, and repository values in `.env.test` or `.env.prod`. The image URLs are derived from those values. Bootstrap the repository first with `terraform apply` using placeholder image values if needed.

Select the vendor with `TF_VAR_cloud_vendor` (or `cloud_vendor` in tfvars). Only `google` is implemented currently; `azure`, `aws`, and `digitalocean` are reserved for future modules.

## Environments

Run Terraform separately from each environment:

```sh
cd terraform/environments/test
set -a
. ../../../.env.test
set +a
terraform init
terraform plan
terraform apply
```

Use `.env.prod` and the corresponding directory for `prod`. The `.env.*` files contain project IDs, image URLs, and deployment-specific values and must not be committed. The `terraform.tfvars.example` files only document the environment-variable workflow.

For remote state, initialize each environment with its own backend configuration, for example:

```sh
terraform init -backend-config=../../test.backend
```

The backend bucket must be created separately before initialization.
