<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Système d'exploitation libre pour agents — Libre comme dans liberté</h3>

<p align="center">
  Agent OS open-source écrit en Rust. 24 crates. 2 100+ tests. Zéro avertissement clippy.
</p>

<p align="center">
  <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pl.md">Polski</a> | <a href="README.fr.md">Français</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Site web</a> &bull;
  <a href="https://docs.librefang.ai">Documentation</a> &bull;
  <a href="../CONTRIBUTING.md">Contribuer</a> &bull;
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

> **Note de traduction.** Ce README en français est un squelette de navigation : titres, sections et liens sont localisés, mais certaines parties techniques détaillées peuvent encore renvoyer à l'anglais. Une traduction complète est prévue en suivi (voir issue #3399). Pour le guide « Premiers pas » entièrement traduit, consultez [getting-started.fr.md](getting-started.fr.md).

## Qu'est-ce que LibreFang ?

LibreFang est un **Système d'exploitation pour agents** — une plateforme complète pour exécuter des agents IA autonomes, construite à partir de zéro en Rust. Pas un framework de chatbot, pas un wrapper Python.

Les frameworks d'agents traditionnels attendent que vous tapiez quelque chose. LibreFang exécute des **agents qui travaillent pour vous** — selon des plannings, 24h/24 et 7j/7, surveillant des cibles, générant des leads, gérant les réseaux sociaux et faisant rapport à votre tableau de bord.

> LibreFang est un fork communautaire de [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) avec une gouvernance ouverte et une politique PR axée sur le merge-first. Voir [GOVERNANCE.md](../GOVERNANCE.md) pour les détails.

<p align="center">
  <img src="../public/assets/dashboard.png" width="800" alt="LibreFang Dashboard" />
</p>

## Démarrage rapide

```bash
# Installation (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# Ou installer via Cargo
cargo install --git https://github.com/librefang/librefang librefang-cli

# Démarrer — initialisation automatique au premier lancement, tableau de bord sur http://localhost:4545
librefang start

# Ou exécuter l'assistant de configuration manuellement pour la sélection interactive du fournisseur
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
<summary><strong>Déploiement Cloud</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands : des agents qui travaillent pour vous

Les **Hands** sont des paquets de capacités autonomes qui s'exécutent indépendamment, selon des plannings, sans prompting. Chaque Hand est défini par un manifeste `HAND.toml`, un prompt système et des fichiers `SKILL.md` optionnels chargés depuis votre `hands_dir` configuré.

Des exemples de définitions de Hands (Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps) sont disponibles dans le [dépôt communautaire des Hands](https://github.com/librefang-registry/hands).

```bash
# Installer un Hand de la communauté, puis :
librefang hand activate researcher   # Commence à travailler immédiatement
librefang hand status researcher     # Vérifier la progression
librefang hand list                  # Voir tous les Hands
```

Créer le vôtre : définissez `HAND.toml` + prompt système + `SKILL.md`. [Guide](https://docs.librefang.ai/agent/skills)

## Architecture

24 crates Rust + xtask, conception modulaire du noyau.

```
librefang-kernel            Orchestration, workflows, métering, RBAC, scheduler, budget
librefang-runtime           Boucle d'agent, exécution d'outils, sandbox WASM, MCP, A2A
librefang-api               140+ endpoints REST/WS/SSE, API compatible OpenAI, dashboard
librefang-channels          45 adaptateurs de messagerie, rate limiting, politiques DM/groupe
librefang-memory            Persistance SQLite, embeddings vectoriels, sessions, compaction
librefang-types             Types core, taint tracking, signature Ed25519, catalogue de modèles
librefang-skills            60 skills intégrées, parser SKILL.md, marketplace FangHub
librefang-hands             Hands autonomes, parser HAND.toml, gestion du cycle de vie
librefang-extensions        25 templates MCP, vault AES-256-GCM, OAuth2 PKCE
librefang-wire              Protocole P2P OFP, authentification mutuelle HMAC-SHA256
librefang-cli               CLI, gestion du daemon, dashboard TUI, mode serveur MCP
librefang-desktop           App native Tauri 2.0 (tray, notifications, raccourcis)
librefang-migrate           Moteur de migration OpenClaw, LangChain, AutoGPT
librefang-http              Constructeur de client HTTP partagé, proxy, repli TLS
librefang-testing           Infrastructure de test : kernel mock, driver LLM mock et utilitaires de test des routes API
librefang-telemetry         Instrumentation des métriques OpenTelemetry + Prometheus pour LibreFang
librefang-llm-driver        Trait du driver LLM et types partagés pour LibreFang
librefang-llm-drivers       Drivers concrets de fournisseurs LLM (anthropic, openai, gemini, …) implémentant le trait librefang-llm-driver
librefang-runtime-mcp       Client MCP (Model Context Protocol) pour le runtime de LibreFang
librefang-kernel-handle     Trait KernelHandle pour les appelants in-process vers le kernel de LibreFang
librefang-runtime-wasm      Sandbox de skills WASM pour le runtime de LibreFang
librefang-kernel-router     Moteur de routage Hand/Template pour le kernel de LibreFang
librefang-runtime-oauth     Flux OAuth (ChatGPT, GitHub Copilot) pour les drivers du runtime de LibreFang
librefang-kernel-metering   Métering des coûts et application des quotas pour le kernel de LibreFang
xtask                       Automatisation de build
```
> **OFP wire est plaintext-by-design.** L'authentification mutuelle HMAC-SHA256 + HMAC
> par message + protection anti-rejeu par nonce couvrent les attaquants *actifs*, mais
> le contenu des trames n'est pas chiffré. Pour la fédération inter-réseaux, exécutez
> OFP derrière un overlay privé (WireGuard, Tailscale, tunnel SSH) ou une couche mTLS
> de service-mesh.
> Détails : [docs.librefang.ai/architecture/ofp-wire](https://docs.librefang.ai/architecture/ofp-wire)

## Fonctionnalités principales

**45 adaptateurs de canaux** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky et 32 autres. [Liste complète](https://docs.librefang.ai/integrations/channels)

**28 fournisseurs LLM** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama et 20 autres. Routage intelligent, fallback automatique, suivi des coûts. [Détails](https://docs.librefang.ai/configuration/providers)

**16 couches de sécurité** — Sandbox WASM, piste d'audit Merkle, taint tracking, signature Ed25519, protection SSRF, zeroization des secrets et plus. [Détails](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth)

**API compatible OpenAI** — Endpoint drop-in `/v1/chat/completions`. 140+ endpoints REST/WS/SSE. [Référence API](https://docs.librefang.ai/integrations/api)

**SDK clients** — Client REST complet avec support du streaming.

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

**Support MCP** — Client et serveur MCP intégrés. Connexion IDE, outils personnalisés, pipelines d'agents. [Détails](https://docs.librefang.ai/integrations/mcp-a2a)

**Protocole A2A** — Support du protocole Agent-to-Agent de Google. Découvrez, communiquez et déléguez des tâches entre systèmes d'agents. [Détails](https://docs.librefang.ai/integrations/mcp-a2a)

**Application desktop** — App native Tauri 2.0 avec system tray, notifications et raccourcis globaux.

**Migration OpenClaw** — `librefang migrate --from openclaw` importe agents, historique, skills et configuration.

## Développement

```bash
cargo build --workspace --lib                            # Build
cargo test --workspace                                   # 2 100+ tests
cargo clippy --workspace --all-targets -- -D warnings    # Zéro avertissement
cargo fmt --all -- --check                               # Vérification du formatage
```

## Comparaison

Voir [Comparaison](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth) pour les benchmarks et la comparaison des fonctionnalités vs OpenClaw, ZeroClaw, CrewAI, AutoGen et LangGraph.

## Documentation

- [Premiers pas (français)](getting-started.fr.md) — Guide d'installation et premier agent
- [Documentation officielle](https://docs.librefang.ai) (en anglais)

## Liens

- [Documentation](https://docs.librefang.ai) &bull; [Référence API](https://docs.librefang.ai/integrations/api) &bull; [Premiers pas](https://docs.librefang.ai/getting-started) &bull; [Dépannage](https://docs.librefang.ai/operations/troubleshooting)
- [Contribuer](../CONTRIBUTING.md) &bull; [Gouvernance](../GOVERNANCE.md) &bull; [Sécurité](../SECURITY.md)
- Discussions : [Q&R](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [Cas d'usage](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [Votes de fonctionnalités](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [Annonces](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Contributeurs

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  Nous accueillons les contributions de toute nature — code, documentation, traductions, rapports de bugs.<br/>
  Consultez le <a href="../CONTRIBUTING.md">Guide de contribution</a> et choisissez une <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a> pour commencer !<br/>
  Vous pouvez aussi visiter le <a href="https://leszek3737.github.io/librefang-WIki/">wiki non officiel</a>, mis à jour avec des informations utiles pour les nouveaux contributeurs.
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">Licence MIT</p>
