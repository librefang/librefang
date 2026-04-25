# LibreFang Observability

Full local telemetry stack for LibreFang: metrics (Prometheus), traces
(Tempo + Jaeger), and logs (Loki + Alloy), all unified in Grafana with
trace ↔ metric ↔ log cross-linking.

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
# 5. Open Jaeger for trace-debug UI
open http://localhost:16686
```

Prometheus scrapes `http://host.docker.internal:4545/api/metrics` every
15 seconds. Alloy tails `${HOME}/.librefang/logs/*.log` and pushes lines
to Loki. The OTel collector receives OTLP traces from the daemon and
fans them out to Tempo, Jaeger, and stdout.

## What runs in the stack

| Service | Port | Purpose |
|---|---|---|
| Grafana | 3000 | Unified UI for metrics / traces / logs |
| Prometheus | 9090 | Metrics store (scrapes daemon `/api/metrics`) |
| OTel Collector | 4317 / 4318 | Receives OTLP from the daemon, fans out |
| Tempo | 3200 | Trace store (Grafana correlation backend) |
| Jaeger | 16686 | Standalone trace-debug UI (waterfall, diff, deps) |
| Loki | 3100 | Log store (queried from Grafana) |
| Alloy | 12345 | Log collector (tails daemon log files) |

Both trace backends are auto-provisioned as Grafana datasources
(`librefang-tempo`, `librefang-jaeger`), so Grafana's Explore page can
query either side. Same `trace_id` flows through both exporters, so a
trace opened in Grafana and the same `trace_id` pasted into the Jaeger
UI return the identical span tree.

Loki is also auto-provisioned (`librefang-loki`). The datasource has
`derivedFields` configured to extract `trace_id="<32-hex>"` from log
lines and turn it into a clickable link that opens the matching trace
in Tempo or Jaeger. The link wiring is in place, but it only lights up
once daemon log lines actually carry the `trace_id` field — that's a
follow-up Rust-side change.

The Jaeger container is **required by the trace pipeline**, not optional:
the collector's `traces` pipeline includes `otlp/jaeger` as an exporter,
so starting the stack without `jaeger` will leave the collector logging
`ConnectionRefused` on every batch. To run a Tempo-only stack, comment
out the `otlp/jaeger` exporter (and remove it from
`service.pipelines.traces.exporters`) in `otel-collector/config.yaml`
and drop the `jaeger` service from `docker-compose.observability.yml`.

## Startup ordering

The compose file uses `depends_on: { condition: service_healthy }` plus
per-service healthchecks (otel-collector `:13133`, Tempo `/ready`,
Jaeger `/`, Loki `/ready`, Prometheus `/-/ready`, Grafana
`/api/health`). This is a deliberate fix for a noisy boot-time race
where the daemon's `BatchSpanProcessor` would log `ConnectionRefused`
against `127.0.0.1:4317` for a few seconds while the collector was
still starting. With healthchecks in place, dependents only start after
their dependencies are actually accepting traffic — the race is gone.

The Jaeger container is **required by the trace pipeline**, not optional:
the collector's `traces` pipeline includes `otlp/jaeger` as an exporter,
so starting the stack without `jaeger` will leave the collector logging
`ConnectionRefused` on every batch. To run a Tempo-only stack, comment
out the `otlp/jaeger` exporter (and remove it from
`service.pipelines.traces.exporters`) in `otel-collector/config.yaml`
and drop the `jaeger` service from `docker-compose.observability.yml`.

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
| `librefang_tokens` | agent, provider, model | gauge | Total tokens consumed |
| `librefang_tokens_input` | agent, provider, model | gauge | Input (prompt) tokens |
| `librefang_tokens_output` | agent, provider, model | gauge | Output (completion) tokens |
| `librefang_tool_calls` | agent, provider, model | gauge | Tool calls made |
| `librefang_llm_calls` | agent, provider, model | gauge | LLM API invocations |

### HTTP (requires `telemetry` feature)

| Metric | Labels | Type | Description |
|--------|--------|------|-------------|
| `librefang_http_requests_total` | method, path, status | counter | HTTP request count |
| `librefang_http_request_duration_seconds` | method, path | histogram | Request latency |

## Dashboards

Four dashboards are bundled in `grafana/dashboards/` and auto-provisioned. Each dashboard includes navigation links to the other three.

### LibreFang Overview (`librefang.json`)
System-level health at a glance: version, uptime, agent counts, active sessions, daily cost, panics/restarts stats. Timeline panels for panics & restarts and active vs total agents.

### LLM & Token Usage (`librefang-llm.json`)
LLM-specific metrics with **template variables** (Agent, Provider, Model) for interactive filtering. Panels: total/input/output token stats, tokens consumed by agent (timeseries), LLM calls by agent (bar), input vs output token breakdown (stacked bar), tokens by provider/model, agent token share (pie), input/output ratio (pie), and tool calls by agent.

### HTTP & API (`librefang-http.json`)
API layer monitoring: request rate by method, latency percentiles (p50/p90/p99), status code distribution, 4xx/5xx error rate, top endpoints by request count, slowest endpoints by p99 latency.

### Cost & Budget (`librefang-cost.json`)
Spending visibility with **template variables** (Agent, Provider, Model) for drill-down. Panels: today's estimated cost (USD), cost trend over time, tokens by agent as cost proxy, token distribution by provider/model (pie), output token ranking per agent (output tokens cost 3-5x more), and input/output cost ratio.

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
