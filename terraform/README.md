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

## Setup

Create separate GCP projects for test and prod, enable billing, install
`gcloud` and Terraform, and authenticate:

```sh
gcloud auth application-default login
cp .env.test.example .env.test
# Set GCP_TEST_PROJECT and GCP_REGION in the environment or Makefile.
make cloud-deploy-test
```

Production uses the separate project and command:

```sh
make cloud-deploy-prod
```

The deploy target builds both images with Cloud Build, pushes them to the
environment's Artifact Registry repository, and runs noninteractive Terraform.
`make cloud-start-test`, `cloud-stop-test`, `cloud-start-prod`, and
`cloud-stop-prod` restore or scale both services to zero. Override
`GCP_TEST_PROJECT`, `GCP_PROD_PROJECT`, `GCP_REGION`, or `GCP_ENV` as needed.
Never put passwords or certificates in Terraform variables or image layers.
