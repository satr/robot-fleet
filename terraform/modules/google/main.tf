resource "google_project_service" "services" {
  for_each = toset([
    "artifactregistry.googleapis.com",
    "compute.googleapis.com",
    "run.googleapis.com",
    "sqladmin.googleapis.com",
    "vpcaccess.googleapis.com",
  ])

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

  depends_on = [google_project_service.services]
}

resource "google_compute_network" "main" {
  project                 = var.project_id
  name                    = "${var.name_prefix}-network"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "main" {
  project       = var.project_id
  name          = "${var.name_prefix}-subnet"
  region        = var.region
  network       = google_compute_network.main.id
  ip_cidr_range = "10.20.0.0/24"
}

resource "google_compute_global_address" "private_services" {
  project       = var.project_id
  name          = "${var.name_prefix}-private-services"
  purpose       = "VPC_PEERING"
  address_type  = "INTERNAL"
  prefix_length = 16
  network       = google_compute_network.main.id
}

resource "google_service_networking_connection" "private_services" {
  network                 = google_compute_network.main.id
  service                 = "servicenetworking.googleapis.com"
  reserved_peering_ranges = [google_compute_global_address.private_services.name]
}

resource "google_sql_database_instance" "postgres" {
  project             = var.project_id
  name                = "${var.name_prefix}-postgres"
  region              = var.region
  database_version    = "POSTGRES_16"
  deletion_protection = var.database_deletion_protection

  settings {
    tier              = var.database_tier
    availability_type = var.environment == "prod" ? "REGIONAL" : "ZONAL"
    disk_type         = "PD_SSD"
    disk_size         = var.environment == "prod" ? 50 : 20
    disk_autoresize   = true

    ip_configuration {
      ipv4_enabled    = false
      private_network = google_compute_network.main.id
    }

    backup_configuration {
      enabled                        = true
      point_in_time_recovery_enabled = var.environment == "prod"
    }
  }

  depends_on = [google_service_networking_connection.private_services]
}

resource "google_sql_database" "robot_fleet" {
  project  = var.project_id
  name     = "robot_fleet"
  instance = google_sql_database_instance.postgres.name
}

resource "random_password" "database" {
  length = 32
}

resource "google_sql_user" "robot_fleet" {
  project  = var.project_id
  name     = "robot_fleet"
  instance = google_sql_database_instance.postgres.name
  password = random_password.database.result
}

resource "google_vpc_access_connector" "serverless" {
  project       = var.project_id
  name          = "${var.name_prefix}-connector"
  region        = var.region
  network       = google_compute_network.main.name
  ip_cidr_range = "10.20.1.0/28"
}

resource "google_compute_address" "mqtt" {
  project = var.project_id
  name    = "${var.name_prefix}-mqtt"
  region  = var.region
}

resource "google_compute_firewall" "mqtt" {
  project = var.project_id
  name    = "${var.name_prefix}-mqtt-ingress"
  network = google_compute_network.main.name

  allow {
    protocol = "tcp"
    ports    = ["1883"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["robot-fleet-mqtt"]
}

resource "google_compute_instance" "mqtt" {
  project      = var.project_id
  name         = "${var.name_prefix}-mqtt"
  zone         = var.zone
  machine_type = var.vm_machine_type
  tags         = ["robot-fleet-mqtt"]

  boot_disk {
    initialize_params {
      image = "debian-cloud/debian-12"
      size  = 20
    }
  }

  network_interface {
    subnetwork = google_compute_subnetwork.main.id
    access_config {
      nat_ip = google_compute_address.mqtt.address
    }
  }

  metadata_startup_script = <<-SCRIPT
    #!/bin/bash
    set -euo pipefail
    apt-get update
    apt-get install -y docker.io
    systemctl enable --now docker
    docker run -d --restart unless-stopped --name mqtt -p 1883:1883 eclipse-mosquitto:2
  SCRIPT
}

resource "google_cloud_run_v2_service" "backend" {
  project  = var.project_id
  name     = "${var.name_prefix}-backend"
  location = var.region

  template {
    vpc_access {
      connector = google_vpc_access_connector.serverless.id
      egress    = "PRIVATE_RANGES_ONLY"
    }

    containers {
      image = var.backend_image
      ports {
        container_port = 8089
      }
      env {
        name  = "DATABASE_URL"
        value = "postgres://robot_fleet:${random_password.database.result}@${google_sql_database_instance.postgres.private_ip_address}:5432/robot_fleet"
      }
      env {
        name  = "MQTT_URL"
        value = "mqtt://${google_compute_address.mqtt.address}:1883"
      }
      env {
        name  = "HTTP_PORT"
        value = "8089"
      }
      env {
        name  = "RUST_LOG"
        value = "info"
      }
    }
  }
}

resource "google_cloud_run_service_iam_member" "backend_public" {
  project  = var.project_id
  location = var.region
  service  = google_cloud_run_v2_service.backend.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

resource "google_cloud_run_v2_service" "web" {
  project  = var.project_id
  name     = "${var.name_prefix}-web"
  location = var.region

  template {
    containers {
      image = var.web_image
      ports {
        container_port = 3001
      }
      env {
        name  = "HOST"
        value = "0.0.0.0"
      }
      env {
        name  = "PORT"
        value = "3001"
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
}

resource "google_cloud_run_service_iam_member" "web_public" {
  project  = var.project_id
  location = var.region
  service  = google_cloud_run_v2_service.web.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

locals {
  robots = {
    robot-01 = "Loader One"
    robot-02 = "Picker Two"
    robot-03 = "Scout Three"
  }
}

resource "google_cloud_run_v2_service" "robots" {
  for_each = local.robots
  project  = var.project_id
  name     = "${var.name_prefix}-${each.key}"
  location = var.region

  template {
    vpc_access {
      connector = google_vpc_access_connector.serverless.id
      egress    = "PRIVATE_RANGES_ONLY"
    }
    containers {
      image = var.robot_image
      env {
        name  = "ROBOT_ID"
        value = each.key
      }
      env {
        name  = "ROBOT_NAME"
        value = each.value
      }
      env {
        name  = "MQTT_URL"
        value = "mqtt://${google_compute_address.mqtt.address}:1883"
      }
      env {
        name  = "RUST_LOG"
        value = "info"
      }
    }
  }
}
