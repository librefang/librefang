<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Freies Agenten-Betriebssystem — Libre bedeutet Freiheit</h3>

<p align="center">
  Open-Source Agent OS in Rust. 24 Crates. 2.100+ Tests. Null Clippy-Warnungen.
</p>

<p align="center">
    <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pl.md">Polski</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Webseite</a> &bull;
  <a href="https://docs.librefang.ai">Dokumentation</a> &bull;
  <a href="../CONTRIBUTING.md">Mitwirken</a> &bull;
  <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/librefang/librefang/ci.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/v/release/librefang/librefang?style=flat-square" alt="Latest Release" />
  <a href="https://discord.gg/DzTYqAZZmc"><img src="https://img.shields.io/discord/1481633471507071129?style=flat-square&logo=discord&label=Discord" alt="Discord" /></a>
  <a href="https://deepwiki.com/librefang/librefang"><img src="https://deepwiki.com/badge.svg" alt="Ask DeepWiki"></a>
</p>

---

## Was ist LibreFang?

LibreFang ist ein **Agenten-Betriebssystem** — eine vollständige Plattform zur Ausführung autonomer KI-Agenten, von Grund auf in Rust gebaut. Kein Chatbot-Framework, kein Python-Wrapper.

Herkömmliche Agenten-Frameworks warten auf Ihre Eingabe. LibreFang führt **Agenten aus, die für Sie arbeiten** — nach Zeitplan, 24/7, Ziele überwachen, Leads generieren, Social Media verwalten und an Ihr Dashboard berichten.

> LibreFang ist ein Community-Fork von [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) mit offener Governance und Merge-First PR-Policy. Siehe [GOVERNANCE.md](../GOVERNANCE.md) für Details.

<p align="center">
  <img src="../public/assets/dashboard.png" width="800" alt="LibreFang Dashboard" />
</p>

## Schnellstart

```bash
# Installieren (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# Oder per Cargo installieren
cargo install --git https://github.com/librefang/librefang librefang-cli

# Starten — initialisiert sich beim ersten Ausführen automatisch, Dashboard unter http://localhost:4545
librefang start

# Oder führen Sie den Setup-Assistenten für eine interaktive Anbieterauswahl manuell aus
# librefang init
```

<details>
<summary><strong>Homebrew</strong></summary>

```bash
brew tap librefang/tap
brew install librefang              # CLI (stable)
brew install --cask librefang       # Desktop (stable)
# Beta/RC channels also available:
# brew install librefang-beta       # or librefang-rc
# brew install --cask librefang-rc  # or librefang-beta
```

</details>

<details>
<summary><strong>Docker</strong></summary>

```bash
docker run -p 4545:4545 ghcr.io/librefang/librefang
```

</details>

<details>
<summary><strong>Cloud-Deployment</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands: Agenten, die für Sie arbeiten

**Hands** sind autonome Fähigkeitspakete, die unabhängig, nach Zeitplänen und ohne Prompting ausgeführt werden. Jeder Hand wird durch ein HAND.toml-Manifest, einen System-Prompt und optionale SKILL.md-Dateien definiert, die geladen werden aus Ihrer konfigurierten `hands_dir`.

Beispielhafte Hand-Definitionen (Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps) sind im [community hands repository](https://github.com/librefang-registry/hands).
```bash
# Einen Community-Hand installieren, dann:
librefang hand activate researcher   # Beginnt sofort zu arbeiten
librefang hand status researcher     # Fortschritt prüfen
librefang hand list                  # Alle Hands anzeigen
```

Eigene Hands erstellen: `HAND.toml` + System-Prompt + `SKILL.md` definieren. [Anleitung](https://docs.librefang.ai/agent/skills)

## Architektur

24 Rust-Crates + xtask, modulares Kernel-Design.

```
librefang-kernel            Orchestrierung, Workflows, Metering, RBAC, Scheduler, Budget
librefang-runtime           Agenten-Loop, 3 LLM-Treiber, 53 Tools, WASM-Sandbox, MCP, A2A
librefang-api               140+ REST/WS/SSE-Endpunkte, OpenAI-kompatible API, Dashboard
librefang-channels          40 Messaging-Adapter, Rate Limiting, DM/Gruppen-Policies
librefang-memory            SQLite-Persistenz, Vektor-Embeddings, Sessions, Komprimierung
librefang-types             Kerntypen, Taint-Tracking, Ed25519-Signierung, Modellkatalog
librefang-skills            60 gebündelte Skills, SKILL.md-Parser, FangHub-Marktplatz
librefang-hands             autonome Hands, HAND.toml-Parser, Lifecycle-Management
librefang-extensions        25 MCP-Templates, AES-256-GCM-Vault, OAuth2 PKCE
librefang-wire              OFP P2P-Protokoll, HMAC-SHA256 gegenseitige Authentifizierung
librefang-cli               CLI, Daemon-Management, TUI-Dashboard, MCP-Servermodus
librefang-desktop           Tauri 2.0 native App (Tray, Benachrichtigungen, Shortcuts)
librefang-migrate           OpenClaw, LangChain, AutoGPT Migrationsengine
librefang-http              Gemeinsamer HTTP-Client-Builder, Proxy, TLS-Fallback
librefang-testing           Testinfrastruktur: Mock-Kernel, Mock-LLM-Driver und Testdienstprogramme für API-Routen
librefang-telemetry         OpenTelemetry + Prometheus Metrik-Instrumentierung für LibreFang
librefang-llm-driver        LLM-Driver-Trait und gemeinsame Typen für LibreFang
librefang-llm-drivers       Konkrete LLM-Provider-Driver (Anthropic, OpenAI, Gemini, …), die das librefang-llm-driver Trait implementieren
librefang-runtime-mcp       MCP (Model Context Protocol) Client für die LibreFang-Runtime
librefang-kernel-handle     KernelHandle-Trait für In-Process-Aufrufer in den LibreFang-Kernel
librefang-runtime-wasm      WASM-Skill-Sandbox für die LibreFang-Runtime
librefang-kernel-router     Hand/Template-Routing-Engine für den LibreFang-Kernel
librefang-runtime-oauth     OAuth-Flows (ChatGPT, GitHub Copilot) für LibreFang-Runtime-Driver
librefang-kernel-metering   Kostenmessung und Durchsetzung von Quoten für den LibreFang-Kernel
xtask                       Build-Automatisierung
```
> **OFP wire is plaintext-by-design.** HMAC-SHA256 mutual auth + per-message
> HMAC + nonce replay protection cover *active* attackers, but frame contents
> are not encrypted. For cross-network federation, run OFP behind a private
> overlay (WireGuard, Tailscale, SSH tunnel) or a service-mesh mTLS layer.
> Details: [docs.librefang.ai/architecture/ofp-wire](https://docs.librefang.ai/architecture/ofp-wire)

## Hauptfunktionen

**45 Kanaladapter** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, E-Mail, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky und 32 weitere. [Vollständige Liste](https://docs.librefang.ai/integrations/channels)

**28 LLM-Anbieter** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama und 20 weitere. Intelligentes Routing, automatisches Fallback, Kostenverfolgung. [Details](https://docs.librefang.ai/configuration/providers)

**16 Sicherheitsschichten** — WASM-Sandbox, Merkle-Auditpfad, Taint-Tracking, Ed25519-Signierung, SSRF-Schutz, Secret-Zeroization und mehr. [Details](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth)

**OpenAI-kompatible API** — Drop-in `/v1/chat/completions`-Endpunkt. 140+ REST/WS/SSE-Endpunkte. [API-Referenz](https://docs.librefang.ai/integrations/api)

**Client-SDKs** — Vollständiger REST-Client mit Streaming-Unterstützung.

```javascript
// JavaScript/TypeScript
npm install @librefang/sdk
const { LibreFang } = require("@librefang/sdk");
const client = new LibreFang("http://localhost:4545");
const agent = await client.agents.create({ template: "assistant" });
const reply = await client.agents.message(agent.id, "Hello!");
```

```python
# Python
pip install librefang
from librefang import Client
client = Client("http://localhost:4545")
agent = client.agents.create(template="assistant")
reply = client.agents.message(agent["id"], "Hello!")
```

```rust
// Rust
cargo add librefang
use librefang::LibreFang;
let client = LibreFang::new("http://localhost:4545");
let agent = client.agents().create(CreateAgentRequest { template: Some("assistant".into()), .. }).await?;
```

```go
// Go
go get github.com/librefang/librefang/sdk/go
import "github.com/librefang/librefang/sdk/go"
client := librefang.New("http://localhost:4545")
agent, _ := client.Agents.Create(map[string]interface{}{"template": "assistant"})
```

**MCP-Unterstützung** — Integrierter MCP-Client und -Server. IDE-Anbindung, benutzerdefinierte Tools, Agenten-Pipelines. [Details](https://docs.librefang.ai/integrations/mcp-a2a)

**A2A-Protokoll** — Google Agent-to-Agent Protokollunterstützung. Agentensysteme entdecken, kommunizieren und Aufgaben delegieren. [Details](https://docs.librefang.ai/integrations/mcp-a2a)

**Desktop-App** — Tauri 2.0 native App mit Systemtray, Benachrichtigungen und globalen Shortcuts.

**OpenClaw-Migration** — `librefang migrate --from openclaw` importiert Agenten, Verlauf, Skills und Konfiguration.

## Entwicklung

```bash
cargo build --workspace --lib                            # Build
cargo test --workspace                                   # 2.100+ Tests
cargo clippy --workspace --all-targets -- -D warnings    # Null Warnungen
cargo fmt --all -- --check                               # Formatprüfung
```

## Vergleich

Siehe [Vergleich](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth) für Benchmarks und Feature-Vergleich vs OpenClaw, ZeroClaw, CrewAI, AutoGen und LangGraph.

## Links

- [Dokumentation](https://docs.librefang.ai) &bull; [API-Referenz](https://docs.librefang.ai/integrations/api) &bull; [Erste Schritte](https://docs.librefang.ai/getting-started) &bull; [Fehlerbehebung](https://docs.librefang.ai/operations/troubleshooting)
- [Mitwirken](../CONTRIBUTING.md) &bull; [Governance](../GOVERNANCE.md) &bull; [Sicherheit](../SECURITY.md)
- Diskussionen: [Q&A](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [Anwendungsfälle](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [Feature-Abstimmungen](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [Ankündigungen](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Mitwirkende

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  Wir freuen uns über Beiträge jeder Art — Code, Dokumentation, Übersetzungen, Fehlerberichte.<br/>
  Schau dir den <a href="../CONTRIBUTING.md">Leitfaden für Mitwirkende</a> an und wähle ein <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a> zum Einstieg!<br/>
  Du kannst auch das <a href="https://leszek3737.github.io/librefang-WIki/">inoffizielle Wiki</a> besuchen, das mit hilfreichen Informationen für neue Mitwirkende aktualisiert wird.
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">MIT-Lizenz</p>
