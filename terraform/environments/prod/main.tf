terraform {
  backend "gcs" {}
}

module "environment" {
  source = "../.."

  cloud_vendor                 = var.cloud_vendor
  environment                  = "prod"
  gcp_project_id               = var.gcp_project_id
  gcp_region                   = var.gcp_region
  gcp_zone                     = var.gcp_zone
  name_prefix                  = "robot-fleet-prod"
  backend_image                = var.backend_image
  web_image                    = var.web_image
  robot_image                  = var.robot_image
  database_tier                = var.database_tier
  vm_machine_type              = var.vm_machine_type
  database_deletion_protection = true
}

output "backend_url" { value = module.environment.backend_url }
output "web_url" { value = module.environment.web_url }
output "mqtt_public_ip" { value = module.environment.mqtt_public_ip }
