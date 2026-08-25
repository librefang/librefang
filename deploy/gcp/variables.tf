variable "project_id" {
  description = "GCP project ID"
  type        = string

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{4,28}[a-z0-9]$", var.project_id))
    error_message = "project_id must be a valid 6-30 character GCP project ID."
  }
}

variable "region" {
  description = "GCP region"
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "GCP zone"
  type        = string
  default     = "us-central1-a"
}

variable "ssh_pub_key_path" {
  description = "Path to SSH public key"
  type        = string
  default     = "~/.ssh/id_rsa.pub"
}

variable "allowed_source_cidr" {
  description = "Trusted IPv4 CIDR allowed to reach SSH and LibreFang (for example, 203.0.113.10/32)"
  type        = string

  validation {
    condition     = can(cidrhost(var.allowed_source_cidr, 0)) &&
      var.allowed_source_cidr != "0.0.0.0/0" &&
      var.allowed_source_cidr != "::/0"
    error_message = "allowed_source_cidr must be a valid restricted CIDR; public 0.0.0.0/0 and ::/0 are forbidden."
  }
}

variable "librefang_api_key" {
  description = "Bearer key required to access the public LibreFang listener"
  type        = string
  sensitive   = true

  validation {
    condition     = length(var.librefang_api_key) >= 32
    error_message = "librefang_api_key must contain at least 32 characters."
  }
}

variable "librefang_version" {
  description = "LibreFang release tag (e.g. v2026.7.31), or 'latest' to opt into a floating release"
  type        = string
  default     = "v2026.7.31"
}

variable "groq_api_key" {
  description = "Groq API key"
  type        = string
  default     = ""
  sensitive   = true
}

variable "openai_api_key" {
  description = "OpenAI API key"
  type        = string
  default     = ""
  sensitive   = true
}

variable "anthropic_api_key" {
  description = "Anthropic API key"
  type        = string
  default     = ""
  sensitive   = true
}
