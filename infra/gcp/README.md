# Deploy LibreFang to GCP (Free Tier)

Deploy LibreFang on a GCP `e2-micro` VM with 30 GB persistent storage — **free forever** within GCP's always-free tier.

## Prerequisites

- [GCP account](https://cloud.google.com/free) with a project
- [gcloud CLI](https://cloud.google.com/sdk/docs/install) installed and authenticated
- [Terraform](https://developer.hashicorp.com/terraform/install) >= 1.5
- At least one LLM API key (Groq, OpenAI, or Anthropic)

## Deploy

```bash
cd infra/gcp

# Authenticate with GCP
gcloud auth application-default login

# Configure
cp terraform.tfvars.example terraform.tfvars
# Edit terraform.tfvars with your project_id and API keys

# Deploy (~2 minutes)
terraform init
terraform apply
```

Terraform will output:
- `dashboard_url` — open in browser to access LibreFang
- `ssh_command` — SSH into the VM

## Verify

```bash
# Wait ~1 minute for cloud-init to finish, then:
curl http://<external_ip>:4545/api/health
```

## Teardown

```bash
terraform destroy
```

## Architecture

```
┌─────────────────────────────────┐
│         GCP Free Tier           │
│                                 │
│  ┌───────────────────────────┐  │
│  │   e2-micro VM (0.25 vCPU) │  │
│  │   30 GB pd-standard disk  │  │
│  │                           │  │
│  │   librefang (systemd)     │  │
│  │   :4545 ← dashboard/API  │  │
│  └───────────────────────────┘  │
│                                 │
│  Firewall: SSH(22) + HTTP(4545) │
└─────────────────────────────────┘
```

## Cost

| Resource | Free Tier Limit | Usage |
|----------|----------------|-------|
| e2-micro | 1 instance/month | 1 |
| Standard disk | 30 GB/month | 30 GB |
| Egress | 1 GB/month | minimal |

> **$0/month** within free tier limits. Only egress beyond 1 GB/month is billed.
