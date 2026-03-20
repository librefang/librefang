terraform {
  required_version = ">= 1.5"
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = "~> 5.0"
    }
  }
}

provider "google" {
  project = var.project_id
  region  = var.region
  zone    = var.zone
}

# --- Network ---

resource "google_compute_network" "librefang" {
  name                    = "librefang-vpc"
  auto_create_subnetworks = false
}

resource "google_compute_subnetwork" "librefang" {
  name          = "librefang-subnet"
  ip_cidr_range = "10.0.1.0/24"
  network       = google_compute_network.librefang.id
}

# --- Firewall ---

resource "google_compute_firewall" "allow_ssh" {
  name    = "librefang-allow-ssh"
  network = google_compute_network.librefang.name

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["librefang"]
}

resource "google_compute_firewall" "allow_http" {
  name    = "librefang-allow-http"
  network = google_compute_network.librefang.name

  allow {
    protocol = "tcp"
    ports    = ["4545"]
  }

  source_ranges = ["0.0.0.0/0"]
  target_tags   = ["librefang"]
}

# --- Compute ---

resource "google_compute_instance" "librefang" {
  name         = "librefang"
  machine_type = "e2-micro"
  tags         = ["librefang"]

  boot_disk {
    initialize_params {
      image = "ubuntu-os-cloud/ubuntu-2404-lts-amd64"
      size  = 30
      type  = "pd-standard"
    }
  }

  network_interface {
    subnetwork = google_compute_subnetwork.librefang.id
    access_config {} # ephemeral public IP
  }

  metadata = {
    ssh-keys  = "librefang:${file(pathexpand(var.ssh_pub_key_path))}"
    user-data = templatefile("${path.module}/cloud-init.yml.tpl", {
      librefang_version = var.librefang_version
      groq_api_key      = var.groq_api_key
      openai_api_key    = var.openai_api_key
      anthropic_api_key = var.anthropic_api_key
    })
  }

  labels = {
    app = "librefang"
  }
}
