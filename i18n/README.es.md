<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Sistema Operativo de Agentes Libre — Libre como en Libertad</h3>

<p align="center">
  Agent OS de código abierto construido en Rust. 14 crates. 2,100+ tests. Cero advertencias de clippy.
</p>

<p align="center">
    <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pl.md">Polski</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Sitio web</a> &bull;
  <a href="https://docs.librefang.ai">Documentación</a> &bull;
  <a href="../CONTRIBUTING.md">Contribuir</a> &bull;
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

## ¿Qué es LibreFang?

LibreFang es un **Sistema Operativo de Agentes** — una plataforma completa para ejecutar agentes de IA autónomos, construida desde cero en Rust. No es un framework de chatbot, no es un wrapper de Python.

Los frameworks de agentes tradicionales esperan tu entrada. LibreFang ejecuta **agentes que trabajan para ti** — según horarios, 24/7, monitorizando objetivos, generando leads, gestionando redes sociales e informando a tu dashboard.

> LibreFang es un fork comunitario de [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) con gobernanza abierta y política de merge-first para PRs. Ver [GOVERNANCE.md](../GOVERNANCE.md) para detalles.

<p align="center">
  <img src="../public/assets/dashboard.png" width="800" alt="LibreFang Dashboard" />
</p>

## Inicio Rápido

```bash
# Instalar (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# O instalar con Cargo
cargo install --git https://github.com/librefang/librefang librefang-cli

# Iniciar — se inicializa automáticamente en la primera ejecución, panel de control en http://localhost:4545
librefang start

# O ejecute el asistente de configuración manualmente para la selección interactiva de proveedores
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
<summary><strong>Despliegue en la Nube</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands: Agentes que Trabajan para Ti

**Hands** son paquetes de capacidades autónomas que se ejecutan de forma independiente, en horarios programados y sin necesidad de prompts. Cada Hand se define mediante un manifiesto HAND.toml, un prompt del sistema y archivos SKILL.md opcionales cargados desde su configurada `hands_dir`.

Ejemplos de definiciones de Hands (Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps) están disponibles en el [repositorio de Hands de la comunidad](https://github.com/librefang/hands).

```bash
# Instala un Hand de la comunidad, luego:
librefang hand activate researcher   # Comienza a trabajar inmediatamente
librefang hand status researcher     # Ver progreso
librefang hand list                  # Ver todos los Hands
```

Crea el tuyo: define un `HAND.toml` + prompt de sistema + `SKILL.md`. [Guía](https://docs.librefang.ai/agent/skills)

## Arquitectura

24 crates de Rust + xtask, diseño de kernel modular.

```
librefang-kernel            Orquestación, workflows, medición, RBAC, planificador, presupuesto
librefang-runtime           Bucle de agente, 3 drivers LLM, 53 herramientas, sandbox WASM, MCP, A2A
librefang-api               140+ endpoints REST/WS/SSE, API compatible con OpenAI, dashboard
librefang-channels          40 adaptadores de mensajería, limitación de tasa, políticas DM/grupo
librefang-memory            Persistencia SQLite, embeddings vectoriales, sesiones, compactación
librefang-types             Tipos core, seguimiento de taint, firma Ed25519, catálogo de modelos
librefang-skills            60 skills incluidos, parser SKILL.md, marketplace FangHub
librefang-hands             Hands autónomos, parser HAND.toml, gestión de ciclo de vida
librefang-extensions        25 plantillas MCP, vault AES-256-GCM, OAuth2 PKCE
librefang-wire              Protocolo P2P OFP, autenticación mutua HMAC-SHA256
librefang-cli               CLI, gestión de daemon, dashboard TUI, modo servidor MCP
librefang-desktop           App nativa Tauri 2.0 (bandeja, notificaciones, atajos)
librefang-migrate           Motor de migración OpenClaw, LangChain, AutoGPT
librefang-http              Constructor de cliente HTTP compartido, proxy, respaldo TLS (TLS fallback)
librefang-testing           Infraestructura de pruebas: kernel simulado (mock), driver de LLM simulado y utilidades de prueba de rutas API
librefang-telemetry         Instrumentación de métricas de OpenTelemetry + Prometheus para LibreFang
librefang-llm-driver        Trait de driver LLM y tipos compartidos para LibreFang
librefang-llm-drivers       Drivers concretos de proveedores de LLM (anthropic, openai, gemini, …) que implementan el trait librefang-llm-driver
librefang-runtime-mcp       Cliente MCP (Model Context Protocol) para el runtime de LibreFang
librefang-kernel-handle     Trait KernelHandle para llamadores en proceso (in-process) hacia el kernel de LibreFang
librefang-runtime-wasm      Sandbox de habilidades (skills) WASM para el runtime de LibreFang
librefang-kernel-router     Motor de enrutamiento de Hand/Template para el kernel de LibreFang
librefang-runtime-oauth     Flujos OAuth (ChatGPT, GitHub Copilot) para los drivers del runtime de LibreFang
librefang-kernel-metering   Medición de costos y aplicación de cuotas para el kernel de LibreFang
xtask                       Automatización de build
```

> **OFP wire es plaintext-by-design.** Autenticación mutua HMAC-SHA256 + HMAC
> por mensaje + protección contra repetición nonce cubren a los atacantes *activos*, pero los contenidos de los frames
> no están encriptados. Para cross-network federation, ejecute OFP detrás de un overlay
> privado (WireGuard, Tailscale, túnel SSH) o una capa mTLS de service-mesh.
> Detalles: [docs.librefang.ai/architecture/ofp-wire](https://docs.librefang.ai/architecture/ofp-wire)

## Características Principales

**57 Adaptadores de Canal** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky y 44 más. [Lista completa](https://docs.librefang.ai/integrations/channels)

**28 Proveedores LLM** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama y 20 más. Enrutamiento inteligente, fallback automático, seguimiento de costos. [Detalles](https://docs.librefang.ai/configuration/providers)

**16 Capas de Seguridad** — Sandbox WASM, auditoría Merkle, seguimiento de taint, firma Ed25519, protección SSRF, zeroización de secretos y más. [Detalles](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth)

**API Compatible con OpenAI** — Endpoint drop-in `/v1/chat/completions`. 140+ endpoints REST/WS/SSE. [Referencia API](https://docs.librefang.ai/integrations/api)

**SDKs de Cliente** — Cliente REST completo con soporte de streaming.

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

**Soporte MCP** — Cliente y servidor MCP integrados. Conecta con IDEs, extiende con herramientas personalizadas, compone pipelines de agentes. [Detalles](https://docs.librefang.ai/integrations/mcp-a2a)

**Protocolo A2A** — Soporte del protocolo Agent-to-Agent de Google. Descubre, comunica y delega tareas entre sistemas de agentes. [Detalles](https://docs.librefang.ai/integrations/mcp-a2a)

**App de Escritorio** — App nativa Tauri 2.0 con bandeja del sistema, notificaciones y atajos globales.

**Migración desde OpenClaw** — `librefang migrate --from openclaw` importa agentes, historial, skills y configuración.

## Desarrollo

```bash
cargo build --workspace --lib                            # Build
cargo test --workspace                                   # 2,100+ tests
cargo clippy --workspace --all-targets -- -D warnings    # Cero advertencias
cargo fmt --all -- --check                               # Verificar formato
```

## Comparación

Ver [Comparación](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth) para benchmarks y comparación de características vs OpenClaw, ZeroClaw, CrewAI, AutoGen y LangGraph.

## Enlaces

- [Documentación](https://docs.librefang.ai) &bull; [Referencia API](https://docs.librefang.ai/integrations/api) &bull; [Guía de Inicio](https://docs.librefang.ai/getting-started) &bull; [Solución de Problemas](https://docs.librefang.ai/operations/troubleshooting)
- [Contribuir](../CONTRIBUTING.md) &bull; [Gobernanza](../GOVERNANCE.md) &bull; [Seguridad](../SECURITY.md)
- Discusiones: [Q&A](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [Casos de Uso](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [Votaciones](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [Anuncios](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Contribuidores

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  Damos la bienvenida a contribuciones de todo tipo — código, documentación, traducciones, reportes de bugs.<br/>
  Consulta la <a href="../CONTRIBUTING.md">Guía de Contribución</a> y elige un <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a> para empezar.<br/>
  También puedes visitar la <a href="https://leszek3737.github.io/librefang-WIki/">wiki no oficial</a>, que se actualiza con información útil para nuevos contribuidores.
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">Licencia MIT</p>
