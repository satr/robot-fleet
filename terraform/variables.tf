variable "cloud_vendor" {
  description = "Cloud vendor selected through TF_VAR_cloud_vendor."
  type        = string
  default     = "google"

  validation {
    condition     = contains(["google", "azure", "aws", "digitalocean"], lower(var.cloud_vendor))
    error_message = "cloud_vendor must be google, azure, aws, or digitalocean."
  }
}

variable "environment" {
  type        = string
  description = "Deployment environment."

  validation {
    condition     = contains(["test", "prod"], var.environment)
    error_message = "environment must be test or prod."
  }
}

variable "gcp_project_id" {
  type        = string
  description = "Existing Google Cloud project ID, for example robot-fleet-alice."

  validation {
    condition     = can(regex("^robot-fleet-[a-z0-9-]+$", var.gcp_project_id))
    error_message = "gcp_project_id must start with robot-fleet- and contain lowercase characters, numbers, or hyphens."
  }
}

variable "gcp_region" {
  type        = string
  description = "Google Cloud region used for Cloud Run and Artifact Registry."
  default     = "us-central1"
}

variable "name_prefix" {
  type        = string
  description = "Prefix used for resource names."
}

variable "backend_image" { type = string }
variable "web_image" { type = string }
variable "mqtt_image" { type = string }
variable "postgres_image" { type = string }
variable "postgres_username" {
  type      = string
  sensitive = true
}
variable "postgres_password" {
  type      = string
  sensitive = true
}
variable "mqtt_username" {
  type      = string
  sensitive = true
}
variable "mqtt_password" {
  type      = string
  sensitive = true
}
variable "image_tag" {
  type        = string
  description = "Docker image tag used when backend_image and web_image are not explicitly set."
  default     = "latest"
}
variable "min_instances" {
  type    = number
  default = 0
}
