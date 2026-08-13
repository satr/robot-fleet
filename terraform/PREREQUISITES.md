# Google Cloud deployment prerequisites

This guide covers the one-time setup required before deploying Robot Fleet to
Google Cloud. The deployment uses Cloud Run, Artifact Registry, Cloud Build,
Cloud Storage, and IAM.

## Install required tools

Install and make sure these commands are available:

```sh
gcloud version
terraform version
make --version
```

The local deployment uses `gcloud` and Terraform. Docker is not required for
the cloud build because the images are built by Cloud Build. Docker is still
required for local Docker Compose development.

For GitHub Actions configuration, also install and authenticate the GitHub CLI:

```sh
gh auth login
```

## Authenticate locally

Use an account that can create or administer the target project, link billing,
enable services, and create Cloud Run and Artifact Registry resources:

```sh
gcloud auth login
gcloud auth application-default login
gcloud auth list
gcloud config list
```

Set the quota project for Application Default Credentials after the target
project exists:

```sh
gcloud auth application-default set-quota-project PROJECT_ID
```

For a personal Google account, `gcloud organizations list` may return no
organizations. That is expected; the Makefile creates a project without an
organization.

## Create or select a billing account

List billing accounts available to the authenticated account:

```sh
gcloud billing accounts list
```

Copy the `ACCOUNT_ID` into `GCP_BILLING_ACCOUNT` in `.env.test` or `.env.prod`.
The billing account must be open and usable by the authenticated account.

Alternatively, in Google Cloud Console, open **Billing**, select a billing
account, and grant the deployment account permission to associate projects with
it.

## Configure an environment

Create the local environment file and edit the project, region, billing
account, and image tag:

```sh
cp .env.test.example .env.test
```

At minimum, configure:

```sh
GCP_PROJECT_ID="robot-fleet-00000001"
GCP_REGION="us-central1"
GCP_BILLING_ACCOUNT="000000-000000-000001"
```

`.env.test` and `.env.prod` are local files and are ignored by Git. Do not
commit credentials, passwords, or certificates.

## What the Makefile provisions

Run the deployment target after configuring the environment:

```sh
make cloud-deploy-test
```

The target checks whether the project exists and creates it when necessary,
links the configured billing account, enables the required APIs, creates the
Terraform state bucket and Artifact Registry repository, builds the images with
Cloud Build, and applies Terraform.

For production, configure `.env.prod` and run:

```sh
make cloud-deploy-prod
```

If a project is created manually in Cloud Console instead, ensure billing is
linked and grant the deployment account permissions to enable APIs, create
resources, and use the Terraform state bucket.

## Optional GitHub Actions setup

The repository workflow is manual-only and uses Google Cloud Workload Identity
Federation. After the target project exists and local `gcloud` authentication
is working, create the pool, provider, service account, IAM bindings, and
repository secrets with:

```sh
make cloud-github-auth-test
```

Use `make cloud-github-auth-prod` for production. These targets require `gh`
authentication and use `REPO="owner/repository"` from the environment file.
They configure the following GitHub Actions secrets automatically:

- `GCP_WORKLOAD_IDENTITY_PROVIDER`
- `GCP_DEPLOY_SERVICE_ACCOUNT`

Set these repository variables manually in **GitHub → Settings → Secrets and
variables → Actions → Variables**:

- `GCP_REGION`
- `GCP_TEST_PROJECT`
- `GCP_PROD_PROJECT`

The workflow can then be started from **Actions → Cloud deployment → Run
workflow**. Select a target such as `deploy-test`, `start-test`, or
`stop-test`, and provide the image tag.
