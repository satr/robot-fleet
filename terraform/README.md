# Robot Fleet Terraform

This deploys the test or production application to Google Cloud Run using
Artifact Registry. Each environment has one public backend service and one
public web service, both limited to one instance. The backend image also runs
PostgreSQL and Mosquitto locally: PostgreSQL is ephemeral and is lost whenever
the Cloud Run instance is replaced or scaled to zero. Mosquitto accepts TCP locally and WebSockets on `9001`; the backend proxies the
latter at the service root, which gives external robot simulators a public
`wss://<backend>` endpoint.

This is a testing deployment, not a secure production boundary. Authentication,
authorization, certificates, and broker credentials are intentionally deferred.
Do not send sensitive data to these public services. Cloud Run and Artifact
Registry usage remains subject to quotas and billing; the deployment avoids
Cloud SQL, VMs, VPC connectors, static IPs, Firestore, and Pub/Sub.

## Prerequisites

Complete the [Google Cloud deployment prerequisites](PREREQUISITES.md) before
deploying. It covers required tools, authentication, billing, environment
files, Makefile provisioning, and optional GitHub Actions setup.

## Local deployment

Set a unique project ID for each environment in `.env.test` and `.env.prod`.
For a private Google account, no organization ID is needed. If a project does
not exist, set `GCP_BILLING_ACCOUNT` to a billing account ID that you can use;
the Makefile creates the project without an organization and links that
billing account. Install `gcloud` and Terraform, and authenticate:

```sh
cp .env.test.example .env.test
make cloud-deploy-test
```

Copy the `ACCOUNT_ID` from `gcloud billing accounts list` into
`GCP_BILLING_ACCOUNT` in `.env.test` or `.env.prod`. If no billing accounts are
listed, create or obtain access to one in Google Cloud Console before
deploying.

`cloud-deploy-test` and `cloud-deploy-prod` load the matching local `.env`
file when present, verify both gcloud authentication contexts, create a
missing project, enable the required APIs, and create the Terraform state
bucket from `terraform/test.backend` or `terraform/prod.backend` if it does
not exist. The deploy targets still require a user to authenticate beforehand;
they do not perform an interactive login. `GCP_BILLING_ACCOUNT` is required
only when the target project must be created, but linking it for an existing
project is also supported.

Production uses the separate project and command:

```sh
make cloud-deploy-prod
```

Robot simulators are deployed independently to their own project. Set
`GCP_SIMULATOR_PROJECT_ID` and the backend WebSocket URL in the matching
`.env.test` or `.env.prod`, then run:

```sh
make simulator-deploy-test
make simulator-stop-test
make simulator-start-test
```

The simulator deployment builds one image and runs `robot-01`, `robot-02`, and
`robot-03` as separate Cloud Run services. Use the corresponding `*-prod`
targets for production.

The deploy target builds both images with Cloud Build, pushes them to the
environment's Artifact Registry repository, and runs noninteractive Terraform.
`make cloud-start-test`, `cloud-stop-test`, `cloud-start-prod`, and
`cloud-stop-prod` restore or scale both services to zero. Override
`GCP_TEST_PROJECT`, `GCP_PROD_PROJECT`, `GCP_REGION`, or `GCP_ENV` as needed.
Never put passwords or certificates in Terraform variables or image layers.

## Manual GitHub Actions deployment

The `Cloud deployment` workflow is available only through
`workflow_dispatch`. Select `deploy-test`, `deploy-prod`, `start-test`,
`stop-test`, `start-prod`, or `stop-prod`. Deployment image tags are generated
automatically for each workflow run.

Configure these repository variables:

- `GCP_REGION`
- `GCP_TEST_PROJECT`
- `GCP_PROD_PROJECT`

  Configure these repository secrets for GitHub OIDC authentication:

- `GCP_BILLING_ACCOUNT` (required when a selected project does not exist)
- `GCP_WORKLOAD_IDENTITY_PROVIDER`
- `GCP_DEPLOY_SERVICE_ACCOUNT`

The service account must be allowed to use the configured workload identity
provider and have permissions for project creation (when needed), billing
linking (when needed), Cloud Build, Artifact Registry, Cloud Run, Terraform
state storage, and service enablement.

The `Robot simulator deployment` workflow uses separate repository variables
`GCP_SIMULATOR_TEST_PROJECT`, `GCP_SIMULATOR_PROD_PROJECT`,
`SIMULATOR_MQTT_URL_TEST`, and `SIMULATOR_MQTT_URL_PROD`, plus secrets
`GCP_SIMULATOR_WORKLOAD_IDENTITY_PROVIDER` and
`GCP_SIMULATOR_DEPLOY_SERVICE_ACCOUNT`. Create these credentials with
`make simulator-github-auth-test` and
`make simulator-github-auth-prod`; the simulator service account needs the
same deployment roles in its simulator project.
