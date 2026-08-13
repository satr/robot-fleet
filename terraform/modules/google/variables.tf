variable "environment" { type = string }
variable "project_id" { type = string }
variable "region" { type = string }
variable "name_prefix" { type = string }
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
variable "image_tag" { type = string }
variable "min_instances" { type = number }
