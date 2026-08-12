terraform {
  backend "gcs" {}
}

module "environment" {
  source = "../.."

  cloud_vendor   = var.cloud_vendor
  environment    = "test"
  gcp_project_id = var.gcp_project_id
  gcp_region     = var.gcp_region
  name_prefix    = "robot-fleet-test"
  backend_image  = var.backend_image
  web_image      = var.web_image
  min_instances  = var.min_instances
}

output "artifact_registry_repository" { value = module.environment.artifact_registry_repository }
output "backend_url" { value = module.environment.backend_url }
output "web_url" { value = module.environment.web_url }
