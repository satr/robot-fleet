provider "google" {
  project = var.gcp_project_id
  region  = var.gcp_region
}

module "google" {
  source = "./modules/google"
  count  = var.cloud_vendor == "google" ? 1 : 0

  environment                  = var.environment
  project_id                   = var.gcp_project_id
  region                       = var.gcp_region
  zone                         = var.gcp_zone
  name_prefix                  = var.name_prefix
  backend_image                = var.backend_image
  web_image                    = var.web_image
  robot_image                  = var.robot_image
  database_tier                = var.database_tier
  vm_machine_type              = var.vm_machine_type
  database_deletion_protection = var.database_deletion_protection
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
