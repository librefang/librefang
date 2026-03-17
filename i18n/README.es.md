<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Sistema Operativo de Agentes Libre — Libre como en Libertad</h3>

<p align="center">
  Agent OS de código abierto construido en Rust. 14 crates. 2,100+ tests. Cero advertencias de clippy.
</p>

<p align="center">
  <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Sitio web</a> &bull;
  <a href="https://docs.librefang.ai">Documentación</a> &bull;
  <a href="../CONTRIBUTING.md">Contribuir</a> &bull;
  <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/v/release/librefang/librefang?style=flat-square" alt="Latest Release" />
  <a href="https://discord.gg/DzTYqAZZmc"><img src="https://img.shields.io/discord/1481633471507071129?style=flat-square&logo=discord&label=Discord" alt="Discord" /></a>
</p>

---

## ¿Qué es LibreFang?

LibreFang es un **Sistema Operativo de Agentes** — una plataforma completa para ejecutar agentes de IA autónomos, construida desde cero en Rust. No es un framework de chatbot, no es un wrapper de Python.

Los frameworks de agentes tradicionales esperan tu entrada. LibreFang ejecuta **agentes que trabajan para ti** — según horarios, 24/7, monitorizando objetivos, generando leads, gestionando redes sociales e informando a tu dashboard.

> LibreFang es un fork comunitario de [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) con gobernanza abierta y política de merge-first para PRs. Ver [GOVERNANCE.md](../GOVERNANCE.md) para detalles.

## Inicio Rápido

```bash
# Instalar
cargo install --git https://github.com/librefang/librefang librefang-cli

# Inicializar (te guía en la configuración del proveedor)
librefang init

# Iniciar — dashboard en http://localhost:4545
librefang start
```

<details>
<summary><strong>Homebrew</strong></summary>

```bash
brew tap librefang/tap && brew install librefang
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

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../infra/gcp/README.md)

</details>

## Hands: Agentes que Trabajan para Ti

**Hands** son paquetes de capacidades autónomas preconstruidos que funcionan de forma independiente, según horarios. 14 incluidos:

| Hand | Función |
|------|---------|
| **Researcher** | Investigación profunda — referencias cruzadas, evaluación de credibilidad CRAAP, informes citados |
| **Collector** | Monitorización OSINT — detección de cambios, seguimiento de sentimiento, grafo de conocimiento |
| **Predictor** | Superpredicción — predicciones calibradas con intervalos de confianza |
| **Strategist** | Análisis estratégico — investigación de mercado, inteligencia competitiva, planificación empresarial |
| **Analytics** | Análisis de datos — recopilación, análisis, visualización, informes automáticos |
| **Trader** | Inteligencia de mercado — análisis multi-señal, gestión de riesgos, análisis de cartera |
| **Lead** | Descubrimiento de prospectos — investigación web, scoring, deduplicación, entrega de leads |
| **Twitter** | X/Twitter autónomo — creación de contenido, programación, cola de aprobación |
| **Reddit** | Gestión de Reddit — monitorización de subreddits, publicación, seguimiento de engagement |
| **LinkedIn** | Gestión de LinkedIn — creación de contenido, networking, interacción profesional |
| **Clip** | YouTube a shorts — corta mejores momentos, subtítulos, narración IA |
| **Browser** | Automatización web — basado en Playwright, puerta de aprobación de compras obligatoria |
| **API Tester** | Testing de APIs — descubrimiento de endpoints, validación, pruebas de carga, detección de regresiones |
| **DevOps** | Automatización DevOps — CI/CD, monitorización de infraestructura, respuesta a incidentes |

```bash
librefang hand activate researcher   # Comienza a trabajar inmediatamente
librefang hand status researcher     # Ver progreso
librefang hand list                  # Ver todos los Hands
```

## Arquitectura

14 crates de Rust, diseño de kernel modular.

```
librefang-kernel      Orquestación, workflows, medición, RBAC, planificador, presupuesto
librefang-runtime     Bucle de agente, 3 drivers LLM, 53 herramientas, sandbox WASM, MCP, A2A
librefang-api         140+ endpoints REST/WS/SSE, API compatible con OpenAI, dashboard
librefang-channels    40 adaptadores de mensajería, limitación de tasa, políticas DM/grupo
librefang-memory      Persistencia SQLite, embeddings vectoriales, sesiones, compactación
librefang-types       Tipos core, seguimiento de taint, firma Ed25519, catálogo de modelos
librefang-skills      60 skills incluidos, parser SKILL.md, marketplace FangHub
librefang-hands       14 Hands autónomos, parser HAND.toml, gestión de ciclo de vida
librefang-extensions  25 plantillas MCP, vault AES-256-GCM, OAuth2 PKCE
librefang-wire        Protocolo P2P OFP, autenticación mutua HMAC-SHA256
librefang-cli         CLI, gestión de daemon, dashboard TUI, modo servidor MCP
librefang-desktop     App nativa Tauri 2.0 (bandeja, notificaciones, atajos)
librefang-migrate     Motor de migración OpenClaw, LangChain, AutoGPT
xtask                 Automatización de build
```

## Características Principales

**40 Adaptadores de Canal** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky y 26 más. [Lista completa](../docs/channel-adapters.md)

**27 Proveedores LLM** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama y 20 más. Enrutamiento inteligente, fallback automático, seguimiento de costos. [Detalles](../docs/providers.md)

**16 Capas de Seguridad** — Sandbox WASM, auditoría Merkle, seguimiento de taint, firma Ed25519, protección SSRF, zeroización de secretos y más. [Detalles](../docs/comparison.md#16-security-systems--defense-in-depth)

**API Compatible con OpenAI** — Endpoint drop-in `/v1/chat/completions`. 140+ endpoints REST/WS/SSE. [Referencia API](../docs/api-reference.md)

**Migración desde OpenClaw** — `librefang migrate --from openclaw` importa agentes, historial, skills y configuración.

## Desarrollo

```bash
cargo build --workspace --lib                            # Build
cargo test --workspace                                   # 2,100+ tests
cargo clippy --workspace --all-targets -- -D warnings    # Cero advertencias
cargo fmt --all -- --check                               # Verificar formato
```

## Comparación

Ver [docs/comparison.md](../docs/comparison.md) para benchmarks y comparación de características vs OpenClaw, ZeroClaw, CrewAI, AutoGen y LangGraph.

## Enlaces

- [Documentación](https://docs.librefang.ai) &bull; [Referencia API](../docs/api-reference.md) &bull; [Guía de Inicio](../docs/getting-started.md)
- [Contribuir](../CONTRIBUTING.md) &bull; [Gobernanza](../GOVERNANCE.md) &bull; [Seguridad](../SECURITY.md)
- [Discusiones](https://github.com/librefang/librefang/discussions) — Q&A, casos de uso, votaciones, anuncios &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Contribuidores

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">Licencia MIT</p>
