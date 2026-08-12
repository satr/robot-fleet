output "artifact_registry_repository" {
  value = try(module.google[0].artifact_registry_repository, null)
}

output "backend_url" {
  value = try(module.google[0].backend_url, null)
}

output "web_url" {
  value = try(module.google[0].web_url, null)
}
