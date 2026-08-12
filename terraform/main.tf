provider "google" {
  project = var.gcp_project_id
  region  = var.gcp_region
}

module "google" {
  source = "./modules/google"
  count  = var.cloud_vendor == "google" ? 1 : 0

  environment   = var.environment
  project_id    = var.gcp_project_id
  region        = var.gcp_region
  name_prefix   = var.name_prefix
  backend_image = var.backend_image
  web_image     = var.web_image
  min_instances = var.min_instances
}

resource "terraform_data" "unsupported_vendor" {
  count = var.cloud_vendor == "google" ? 0 : 1

  input = "No Terraform module is implemented for ${var.cloud_vendor} yet. Add one under terraform/modules/${var.cloud_vendor}."

  lifecycle {
    precondition {
      condition     = var.cloud_vendor == "google"
      error_message = "Cloud vendor '${var.cloud_vendor}' is not implemented yet; currently only 'google' is supported."
    }
  }
}
