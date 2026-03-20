output "external_ip" {
  description = "Public IP of the LibreFang VM"
  value       = google_compute_instance.librefang.network_interface[0].access_config[0].nat_ip
}

output "ssh_command" {
  description = "SSH into the VM"
  value       = "ssh librefang@${google_compute_instance.librefang.network_interface[0].access_config[0].nat_ip}"
}

output "dashboard_url" {
  description = "LibreFang dashboard URL"
  value       = "http://${google_compute_instance.librefang.network_interface[0].access_config[0].nat_ip}:4545"
}
