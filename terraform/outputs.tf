output "backend_url" {
  value = try(module.google[0].backend_url, null)
}

output "web_url" {
  value = try(module.google[0].web_url, null)
}

output "mqtt_public_ip" {
  value = try(module.google[0].mqtt_public_ip, null)
}

output "artifact_registry_repository" {
  value = try(module.google[0].artifact_registry_repository, null)
}
