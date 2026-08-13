variable "cloud_vendor" { type = string }
variable "gcp_project_id" { type = string }
variable "gcp_region" { type = string }
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
  type    = string
  default = "latest"
}
variable "min_instances" {
  type    = number
  default = 0
}
