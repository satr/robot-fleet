output "artifact_registry_repository" {
  value = google_artifact_registry_repository.images.name
}

output "backend_url" {
  value = google_cloud_run_v2_service.backend.uri
}

output "web_url" {
  value = google_cloud_run_v2_service.web.uri
}

output "mqtt_url" {
  value = google_cloud_run_v2_service.mqtt.uri
}
