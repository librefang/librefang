---
name: devops-hand-skill
version: "1.0.0"
description: "Expert knowledge for AI DevOps automation -- CI/CD patterns, infrastructure monitoring, deployment strategies, and incident response playbooks"
runtime: prompt_only
---

# DevOps Expert Knowledge

## CI/CD Pipeline Patterns

### GitHub Actions Reference

**Basic workflow structure**:
```yaml
name: CI/CD Pipeline
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: make build
      - name: Test
        run: make test
      - name: Lint
        run: make lint

  deploy:
    needs: build
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    steps:
      - name: Deploy
        run: make deploy
```

**Useful API endpoints**:
```bash
# List workflow runs
curl -s -H "Authorization: Bearer $GITHUB_TOKEN" \
  "https://api.github.com/repos/OWNER/REPO/actions/runs?per_page=10"

# Get workflow run details
curl -s -H "Authorization: Bearer $GITHUB_TOKEN" \
  "https://api.github.com/repos/OWNER/REPO/actions/runs/RUN_ID"

# Re-run failed jobs
curl -s -X POST -H "Authorization: Bearer $GITHUB_TOKEN" \
  "https://api.github.com/repos/OWNER/REPO/actions/runs/RUN_ID/rerun-failed-jobs"
```

### Pipeline Optimization Checklist

- [ ] Cache dependencies (node_modules, .cargo, pip cache)
- [ ] Parallelize independent jobs
- [ ] Use matrix builds for multi-version testing
- [ ] Skip unnecessary steps on non-code changes
- [ ] Use shallow clones for faster checkout
- [ ] Optimize Docker layer caching
- [ ] Run expensive tests only on main branch

---

## Infrastructure Monitoring

### Health Check Patterns

**HTTP endpoint check**:
```bash
curl -s -o /dev/null -w "%{http_code} %{time_total}s" --max-time 10 "$URL"
```

**TCP port check**:
```bash
nc -z -w5 hostname port && echo "UP" || echo "DOWN"
```

**SSL certificate expiry**:
```bash
echo | openssl s_client -servername HOST -connect HOST:443 2>/dev/null | \
  openssl x509 -noout -dates
```

**DNS resolution**:
```bash
dig +short hostname
```

**Disk usage**:
```bash
df -h | grep -v tmpfs
```

**Memory usage**:
```bash
free -h
```

### The Four Golden Signals

| Signal | What to Measure | Alert Threshold |
|--------|----------------|-----------------|
| **Latency** | Request duration | P95 > 500ms |
| **Traffic** | Requests per second | Deviation > 50% from baseline |
| **Errors** | Error rate percentage | > 1% of requests |
| **Saturation** | Resource utilization | CPU/Memory > 80% |

### Docker Monitoring Commands

```bash
# Container status
docker ps --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"

# Resource usage
docker stats --no-stream --format "table {{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.NetIO}}"

# Container logs (last 100 lines)
docker logs --tail 100 CONTAINER_NAME

# Inspect container health
docker inspect --format='{{.State.Health.Status}}' CONTAINER_NAME
```

### Kubernetes Monitoring Commands

```bash
# Pod status across all namespaces
kubectl get pods --all-namespaces -o wide

# Resource usage
kubectl top pods --all-namespaces
kubectl top nodes

# Recent events (errors and warnings)
kubectl get events --sort-by=.lastTimestamp --field-selector type!=Normal

# Pod logs
kubectl logs POD_NAME -n NAMESPACE --tail=100

# Describe failing pod
kubectl describe pod POD_NAME -n NAMESPACE
```

---

## Deployment Strategies

### Blue-Green Deployment
```
1. Run current version on "Blue" environment
2. Deploy new version to "Green" environment
3. Run health checks on Green
4. Switch traffic from Blue to Green
5. Keep Blue as rollback target
6. After validation period, decommission Blue
```

### Canary Deployment
```
1. Deploy new version to small subset (5-10% of traffic)
2. Monitor error rates and latency
3. If healthy, gradually increase traffic (25% -> 50% -> 100%)
4. If problems detected, route all traffic back to old version
```

### Rolling Update
```
1. Update instances one at a time
2. Wait for health check to pass before updating next
3. If any instance fails health check, pause and alert
4. Continue until all instances updated
```

### Deployment Checklist
- [ ] All tests passing in CI
- [ ] Database migrations compatible (backward and forward)
- [ ] Feature flags configured for new features
- [ ] Monitoring and alerting in place
- [ ] Rollback procedure documented and tested
- [ ] On-call engineer notified
- [ ] Change request approved (if required)

---

## Incident Response

### Incident Lifecycle
```
Detection -> Triage -> Mitigation -> Investigation -> Resolution -> Post-mortem
```

### Severity Levels

| Level | Impact | Response Time | Example |
|-------|--------|--------------|---------|
| SEV1 | Full outage | Immediate | Production down |
| SEV2 | Major impact | 15 min | Core feature broken |
| SEV3 | Minor impact | 1 hour | Non-critical feature degraded |
| SEV4 | Low impact | Next business day | Cosmetic issue |

### Incident Response Template
```markdown
# Incident Report: [Title]
**Severity**: SEV[1-4]
**Status**: [Investigating | Mitigated | Resolved]
**Duration**: [Start time] - [End time]

## Timeline
- HH:MM - [Event or action taken]
- HH:MM - [Event or action taken]

## Root Cause
[What caused the incident]

## Impact
[Who was affected and how]

## Mitigation
[What was done to restore service]

## Resolution
[What was done to fix the root cause]

## Action Items
- [ ] [Preventive measure 1]
- [ ] [Preventive measure 2]

## Lessons Learned
[What we can improve]
```

---

## Infrastructure as Code

### Terraform Quick Reference

```bash
# Initialize
terraform init

# Plan changes
terraform plan -out=tfplan

# Apply changes
terraform apply tfplan

# Show current state
terraform show

# Destroy resources (DANGEROUS)
terraform destroy
```

### Docker Compose Quick Reference

```bash
# Start services
docker compose up -d

# Stop services
docker compose down

# View logs
docker compose logs -f SERVICE_NAME

# Rebuild and restart
docker compose up -d --build SERVICE_NAME

# Scale a service
docker compose up -d --scale SERVICE_NAME=3
```

---

## Common Failure Diagnosis Playbooks

### Memory Leak Detection
```bash
# Track memory growth over time
while true; do
  ps aux --sort=-%mem | head -5 | awk '{print strftime("%H:%M:%S"), $2, $4"%", $11}'
  sleep 60
done

# Check for OOM kills
dmesg | grep -i "oom\|killed" | tail -20

# Kubernetes memory pressure
kubectl top pods --sort-by=memory | head -10
```

### DNS Failure Cascade
```bash
# Test DNS resolution
dig hostname +short
dig @8.8.8.8 hostname +short  # Bypass local DNS

# Check /etc/resolv.conf
cat /etc/resolv.conf

# Test from inside a container
kubectl exec -it POD -- nslookup hostname
```

### Database Connection Pool Exhaustion
```bash
# Check active connections (PostgreSQL)
psql -c "SELECT count(*) FROM pg_stat_activity WHERE state = 'active';"
psql -c "SELECT max_conn FROM pg_settings WHERE name = 'max_connections';"

# Check for long-running queries
psql -c "SELECT pid, now() - pg_stat_activity.query_start AS duration, query
         FROM pg_stat_activity WHERE state != 'idle' ORDER BY duration DESC LIMIT 10;"
```

### Certificate Expiry Monitoring
```bash
# Check cert expiry for a list of domains
for domain in api.example.com app.example.com; do
  expiry=$(echo | openssl s_client -servername $domain -connect $domain:443 2>/dev/null | \
    openssl x509 -noout -enddate 2>/dev/null | cut -d= -f2)
  echo "$domain: $expiry"
done
```
