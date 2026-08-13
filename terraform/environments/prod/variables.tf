variable "cloud_vendor" { type = string }
variable "gcp_project_id" { type = string }
variable "gcp_region" { type = string }
variable "backend_image" { type = string }
variable "web_image" { type = string }
variable "image_tag" {
  type    = string
  default = "latest"
}
variable "min_instances" {
  type    = number
  default = 0
}
