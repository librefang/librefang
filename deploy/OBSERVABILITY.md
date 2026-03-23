# LibreFang Observability

Prometheus + Grafana monitoring stack for LibreFang.

## Quick Start

```bash
# 1. Enable Prometheus metrics in LibreFang config (~/.librefang/config.toml)
[telemetry]
prometheus_enabled = true

# 2. Start LibreFang daemon
librefang start

# 3. Start the observability stack
cd deploy
docker compose -f docker-compose.observability.yml up -d

# 4. Open Grafana
open http://localhost:3000    # admin / admin
```

Prometheus scrapes `http://host.docker.internal:4545/api/metrics` every 15 seconds.

## Available Metrics

### System

| Metric | Type | Description |
|--------|------|-------------|
| `librefang_info{version}` | gauge | Build version info |
| `librefang_uptime_seconds` | gauge | Seconds since daemon started |
| `librefang_agents_active` | gauge | Number of running agents |
| `librefang_agents_total` | gauge | Total registered agents |
| `librefang_panics_total` | counter | Supervisor panic count |
| `librefang_restarts_total` | counter | Supervisor restart count |
| `librefang_active_sessions` | gauge | Active dashboard login sessions |
| `librefang_cost_usd_today` | gauge | Estimated total cost for today (USD) |

### LLM & Token Usage (per agent, rolling 1h window)

| Metric | Labels | Type | Description |
|--------|--------|------|-------------|
| `librefang_tokens_total` | agent, provider, model | gauge | Total tokens consumed |
| `librefang_tokens_input_total` | agent, provider, model | gauge | Input (prompt) tokens |
| `librefang_tokens_output_total` | agent, provider, model | gauge | Output (completion) tokens |
| `librefang_tool_calls_total` | agent | gauge | Tool calls made |
| `librefang_llm_calls_total` | agent, provider, model | gauge | LLM API invocations |

### HTTP (requires `telemetry` feature)

| Metric | Labels | Type | Description |
|--------|--------|------|-------------|
| `librefang_http_requests_total` | method, path, status | counter | HTTP request count |
| `librefang_http_request_duration_seconds` | method, path | histogram | Request latency |

## Dashboards

Four dashboards are bundled in `grafana/dashboards/` and auto-provisioned:

### LibreFang Overview (`librefang.json`)
System-level health at a glance: version, uptime, agent counts, active sessions, daily cost, panics/restarts stats. Timeline panels for panics & restarts and active vs total agents.

### LLM & Token Usage (`librefang-llm.json`)
LLM-specific metrics: total/input/output token stats, tokens consumed by agent (timeseries), LLM calls by agent (bar), input vs output token breakdown (stacked bar), tokens by provider/model, agent token share (pie), input/output ratio (pie), and tool calls by agent.

### HTTP & API (`librefang-http.json`)
API layer monitoring: request rate by method, latency percentiles (p50/p90/p99), status code distribution, 4xx/5xx error rate, top endpoints by request count, slowest endpoints by p99 latency.

### Cost & Budget (`librefang-cost.json`)
Spending visibility: today's estimated cost (USD), cost trend over time, tokens by agent as cost proxy, token distribution by provider/model (pie), output token ranking per agent (output tokens cost 3-5x more), and input/output cost ratio.

## Configuration

### Prometheus

Edit `prometheus/prometheus.yml` to change the scrape target:

```yaml
scrape_configs:
  - job_name: "librefang"
    metrics_path: /api/metrics
    static_configs:
      - targets: ["host.docker.internal:4545"]
```

For remote deployments, replace `host.docker.internal:4545` with the actual host and port.

### Grafana

- Default credentials: `admin` / `admin`
- Datasource and dashboard are auto-provisioned via `grafana/provisioning/`
- Dashboard is editable in the UI; changes persist in the `grafana-data` Docker volume

## Stopping

```bash
cd deploy
docker compose -f docker-compose.observability.yml down
# To also delete stored data:
docker compose -f docker-compose.observability.yml down -v
```
