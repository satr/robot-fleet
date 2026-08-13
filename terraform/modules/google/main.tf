locals {
  services = toset([
    "artifactregistry.googleapis.com",
    "run.googleapis.com",
  ])
}

resource "google_project_service" "services" {
  for_each           = local.services
  project            = var.project_id
  service            = each.key
  disable_on_destroy = false
}

locals {
  backend_image = var.backend_image != "" ? var.backend_image : format(
    "%s-docker.pkg.dev/%s/%s-images/backend:%s",
    var.region,
    var.project_id,
    var.name_prefix,
    var.image_tag,
  )
  web_image = var.web_image != "" ? var.web_image : format(
    "%s-docker.pkg.dev/%s/%s-images/web:%s",
    var.region,
    var.project_id,
    var.name_prefix,
    var.image_tag,
  )
}

resource "google_artifact_registry_repository" "images" {
  project       = var.project_id
  location      = var.region
  repository_id = "${var.name_prefix}-images"
  description   = "Robot Fleet container images"
  format        = "DOCKER"
  depends_on    = [google_project_service.services]
}

resource "google_cloud_run_v2_service" "backend" {
  name                = "${var.name_prefix}-backend"
  location            = var.region
  project             = var.project_id
  deletion_protection = var.environment == "prod"
  ingress             = "INGRESS_TRAFFIC_ALL"

  template {
    timeout = "3600s"
    scaling {
      min_instance_count = var.min_instances
      max_instance_count = 1
    }
    containers {
      image = local.backend_image
      ports { container_port = 8080 }
      resources {
        limits   = { cpu = "1", memory = "512Mi" }
        cpu_idle = false
      }
      env {
        name  = "DATABASE_URL"
        value = "postgres://robot_fleet:robot_fleet@127.0.0.1:5432/robot_fleet"
      }
      env {
        name  = "MQTT_URL"
        value = "mqtt://127.0.0.1:1883"
      }
      env {
        name  = "HTTP_PORT"
        value = "8080"
      }
      env {
        name  = "RUST_LOG"
        value = "info"
      }
    }
  }
  depends_on = [google_project_service.services]
}

resource "google_cloud_run_v2_service" "web" {
  name                = "${var.name_prefix}-web"
  location            = var.region
  project             = var.project_id
  deletion_protection = var.environment == "prod"
  ingress             = "INGRESS_TRAFFIC_ALL"

  template {
    scaling {
      min_instance_count = var.min_instances
      max_instance_count = 1
    }
    containers {
      image = local.web_image
      ports { container_port = 8080 }
      resources {
        limits   = { cpu = "1", memory = "512Mi" }
        cpu_idle = true
      }
      env {
        name  = "HOST"
        value = "0.0.0.0"
      }
      env {
        name  = "PUBLIC_BACKEND_HTTP_URL"
        value = google_cloud_run_v2_service.backend.uri
      }
      env {
        name  = "PUBLIC_BACKEND_WS_URL"
        value = replace(google_cloud_run_v2_service.backend.uri, "https://", "wss://")
      }
    }
  }
  depends_on = [google_project_service.services]
}

resource "google_cloud_run_service_iam_member" "backend_public" {
  location = var.region
  project  = var.project_id
  service  = google_cloud_run_v2_service.backend.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

resource "google_cloud_run_service_iam_member" "web_public" {
  location = var.region
  project  = var.project_id
  service  = google_cloud_run_v2_service.web.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}
