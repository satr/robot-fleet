locals {
  services = toset([
    "artifactregistry.googleapis.com",
    "secretmanager.googleapis.com",
    "run.googleapis.com",
  ])

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
  mqtt_image = var.mqtt_image != "" ? var.mqtt_image : format(
    "%s-docker.pkg.dev/%s/%s-images/mqtt:%s",
    var.region,
    var.project_id,
    var.name_prefix,
    var.image_tag,
  )
  postgres_image = var.postgres_image != "" ? var.postgres_image : format(
    "%s-docker.pkg.dev/%s/%s-images/postgres:%s",
    var.region,
    var.project_id,
    var.name_prefix,
    var.image_tag,
  )
}

resource "google_project_service" "services" {
  for_each           = local.services
  project            = var.project_id
  service            = each.key
  disable_on_destroy = false
}

resource "google_artifact_registry_repository" "images" {
  project       = var.project_id
  location      = var.region
  repository_id = "${var.name_prefix}-images"
  description   = "Robot Fleet container images"
  format        = "DOCKER"
  depends_on    = [google_project_service.services]
}

resource "google_service_account" "backend" {
  account_id   = replace("${var.name_prefix}-backend", "-", "")
  project      = var.project_id
  display_name = "Robot Fleet backend runtime"
}

resource "google_service_account" "mqtt" {
  account_id   = replace("${var.name_prefix}-mqtt", "-", "")
  project      = var.project_id
  display_name = "Robot Fleet MQTT runtime"
}

resource "google_secret_manager_secret" "postgres_username" {
  secret_id = "${var.name_prefix}-postgres-username"
  project   = var.project_id
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "postgres_password" {
  secret_id = "${var.name_prefix}-postgres-password"
  project   = var.project_id
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "mqtt_username" {
  secret_id = "${var.name_prefix}-mqtt-username"
  project   = var.project_id
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret" "mqtt_password" {
  secret_id = "${var.name_prefix}-mqtt-password"
  project   = var.project_id
  replication {
    auto {}
  }
}

resource "google_secret_manager_secret_version" "postgres_username" {
  secret      = google_secret_manager_secret.postgres_username.id
  secret_data = var.postgres_username
}

resource "google_secret_manager_secret_version" "postgres_password" {
  secret      = google_secret_manager_secret.postgres_password.id
  secret_data = var.postgres_password
}

resource "google_secret_manager_secret_version" "mqtt_username" {
  secret      = google_secret_manager_secret.mqtt_username.id
  secret_data = var.mqtt_username
}

resource "google_secret_manager_secret_version" "mqtt_password" {
  secret      = google_secret_manager_secret.mqtt_password.id
  secret_data = var.mqtt_password
}

resource "google_secret_manager_secret_iam_member" "backend_access" {
  for_each = {
    postgres_username = google_secret_manager_secret.postgres_username.id
    postgres_password = google_secret_manager_secret.postgres_password.id
    mqtt_username     = google_secret_manager_secret.mqtt_username.id
    mqtt_password     = google_secret_manager_secret.mqtt_password.id
  }
  project   = var.project_id
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.backend.email}"
}

resource "google_secret_manager_secret_iam_member" "mqtt_access" {
  for_each = {
    mqtt_username = google_secret_manager_secret.mqtt_username.id
    mqtt_password = google_secret_manager_secret.mqtt_password.id
  }
  project   = var.project_id
  secret_id = each.value
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${google_service_account.mqtt.email}"
}

resource "google_cloud_run_v2_service" "backend" {
  name                = "${var.name_prefix}-backend"
  location            = var.region
  project             = var.project_id
  deletion_protection = var.environment == "prod"
  ingress             = "INGRESS_TRAFFIC_ALL"

  template {
    timeout         = "3600s"
    service_account = google_service_account.backend.email
    scaling {
      min_instance_count = var.min_instances
      max_instance_count = 1
    }

    containers {
      name  = "backend"
      image = local.backend_image
      ports { container_port = 8080 }
      depends_on = ["postgres"]
      resources {
        limits   = { cpu = "1", memory = "512Mi" }
        cpu_idle = false
      }
      env {
        name  = "HTTP_PORT"
        value = "8080"
      }
      env {
        name  = "POSTGRES_HOST"
        value = "127.0.0.1"
      }
      env {
        name  = "POSTGRES_PORT"
        value = "5432"
      }
      env {
        name  = "POSTGRES_DB"
        value = "robot_fleet"
      }
      env {
        name = "POSTGRES_USER"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.postgres_username.id
            version = "latest"
          }
        }
      }
      env {
        name = "POSTGRES_PASSWORD"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.postgres_password.id
            version = "latest"
          }
        }
      }
      env {
        name  = "MQTT_URL"
        value = replace(google_cloud_run_v2_service.mqtt.uri, "https://", "wss://")
      }
      env {
        name = "MQTT_USERNAME"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.mqtt_username.id
            version = "latest"
          }
        }
      }
      env {
        name = "MQTT_PASSWORD"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.mqtt_password.id
            version = "latest"
          }
        }
      }
      env {
        name  = "RUST_LOG"
        value = "info"
      }
    }

    containers {
      name  = "postgres"
      image = local.postgres_image
      env {
        name = "POSTGRES_USER"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.postgres_username.id
            version = "latest"
          }
        }
      }
      env {
        name = "POSTGRES_PASSWORD"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.postgres_password.id
            version = "latest"
          }
        }
      }
      env {
        name  = "POSTGRES_DB"
        value = "robot_fleet"
      }
      startup_probe {
        tcp_socket {
          port = 5432
        }
        initial_delay_seconds = 5
        period_seconds        = 5
        failure_threshold     = 30
      }
      resources {
        limits   = { cpu = "1", memory = "1Gi" }
        cpu_idle = false
      }
    }
  }
  depends_on = [
    google_project_service.services,
    google_secret_manager_secret_iam_member.backend_access,
  ]
}

resource "google_cloud_run_v2_service" "mqtt" {
  name                = "${var.name_prefix}-mqtt"
  location            = var.region
  project             = var.project_id
  deletion_protection = var.environment == "prod"
  ingress             = "INGRESS_TRAFFIC_ALL"

  template {
    timeout         = "3600s"
    service_account = google_service_account.mqtt.email
    scaling {
      min_instance_count = var.min_instances
      max_instance_count = 1
    }
    containers {
      image = local.mqtt_image
      ports { container_port = 8080 }
      env {
        name = "MQTT_USERNAME"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.mqtt_username.id
            version = "latest"
          }
        }
      }
      env {
        name = "MQTT_PASSWORD"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.mqtt_password.id
            version = "latest"
          }
        }
      }
      resources {
        limits   = { cpu = "1", memory = "512Mi" }
        cpu_idle = false
      }
    }
  }
  depends_on = [
    google_project_service.services,
    google_secret_manager_secret_version.mqtt_username,
    google_secret_manager_secret_version.mqtt_password,
    google_secret_manager_secret_iam_member.mqtt_access,
  ]
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

resource "google_cloud_run_service_iam_member" "mqtt_public" {
  location = var.region
  project  = var.project_id
  service  = google_cloud_run_v2_service.mqtt.name
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
