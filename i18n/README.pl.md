<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Wolnościowy System Operacyjny Agentów — Wolny, a nie tylko darmowy (Free as in Freedom)</h3>

<p align="center">
  Agentowy system operacyjny (Agent OS) typu open-source napisany w języku Rust. 24 paczek (crates). Ponad 2100 testów. Zero ostrzeżeń lintera Clippy.
</p>

<p align="center">
    <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a> | <a href="README.pl.md">Polski</a> | <a href="README.fr.md">Français</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Strona WWW</a> &bull;
  <a href="https://docs.librefang.ai">Dokumentacja</a> &bull;
  <a href="../CONTRIBUTING.md">Współtworzenie</a> &bull;
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

## Czym jest LibreFang?

LibreFang to **System Operacyjny dla Agentów (Agent Operating System)** — pełna platforma do uruchamiania autonomicznych agentów AI, zbudowana od podstaw w języku Rust. To nie jest kolejny framework dla chatbotów ani wrapper na Pythona.

Tradycyjne frameworki agentowe czekają, aż coś wpiszesz. LibreFang uruchamia **agentów, którzy pracują dla Ciebie** — zgodnie z harmonogramami, 24/7, monitorując cele, generując leady, zarządzając mediami społecznościowymi i raportując do Twojego dashboardu.

> LibreFang to społecznościowy fork projektu [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang) charakteryzujący się otwartym zarządzaniem (open governance) oraz polityką PR opartą na zasadzie merge-first. Szczegóły znajdziesz w pliku [GOVERNANCE.md](../GOVERNANCE.md).

<p align="center">
  <img src="../public/assets/dashboard.png" width="800" alt="LibreFang Dashboard" />
</p>

## Szybki start

```bash
# # Instalacja (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# # Lub zainstaluj przez Cargo
cargo install --git https://github.com/librefang/librefang librefang-cli

# Start — automatyczna inicjalizacja przy pierwszym uruchomieniu, panel nawigacyjny pod adresem http://localhost:4545
librefang start

# Lub uruchom kreatora konfiguracji ręcznie w celu interaktywnego wyboru dostawcy
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
<summary><strong>Wdrożenie w Chmurze (Cloud Deploy)</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands: Agenci, którzy pracują dla Ciebie

**Hands** to pakiety autonomicznych możliwości, które działają niezależnie, zgodnie z harmonogramami, bez konieczności wprowadzania promptów. Każdy Hand jest definiowany przez manifest `HAND.toml`, prompt systemowy oraz opcjonalne pliki `SKILL.md` ładowane z Twojego skonfigurowanego `hands_dir`.

Przykładowe definicje Hand (Researcher, Collector, Predictor, Strategist, Analytics, Trader, Lead, Twitter, Reddit, LinkedIn, Clip, Browser, API Tester, DevOps) są dostępne w [repozytorium społecznościowym dla Hands](https://github.com/librefang/hands).

```bash
# Zainstaluj społecznościowy Hand, a następnie:
librefang hand activate researcher   # Rozpoczyna pracę natychmiast
librefang hand status researcher     # Sprawdź postęp
librefang hand list                  # Wyświetl wszystkie Hands
```

Zbuduj własnego: zdefiniuj `HAND.toml` + prompt systemowy + `SKILL.md`. [Przewodnik](https://docs.librefang.ai/agent/skills)

## Architektura

24 paczek (crates) w Rust + xtask, modułowa architektura jądra (kernel).

```
librefang-kernel            Orkiestracja, przepływy pracy, opomiarowanie, RBAC, scheduler, budżet
librefang-runtime           Pętla agenta, wykonywanie narzędzi, piaskownica WASM, MCP, A2A
librefang-api               140+ endpointów REST/WS/SSE, API kompatybilne z OpenAI, dashboard
librefang-channels          45 adapterów komunikacyjnych z limitowaniem liczby żądań, polityki DM/grupowe
librefang-memory            Persystencja SQLite, osadzenia wektorowe, sesje, kompakcja
librefang-types             Typy podstawowe, taint tracking, podpisywanie Ed25519, katalog modeli
librefang-skills            60 wbudowanych umiejętności, parser SKILL.md, marketplace FangHub
librefang-hands             autonomicznych Hands, parser HAND.toml, zarządzanie cyklem życia
librefang-extensions        25 szablonów MCP, skarbiec AES-256-GCM, OAuth2 PKCE
librefang-wire              Protokół OFP P2P, wzajemne uwierzytelnianie HMAC-SHA256
librefang-cli               CLI, zarządzanie demonem, dashboard TUI, tryb serwera MCP
librefang-desktop           Natywna aplikacja Tauri 2.0 (zasobnik systemowy, powiadomienia, skróty)
librefang-migrate           Silnik migracji OpenClaw, LangChain i AutoGPT
librefang-http              Współdzielony kreator klienta HTTP, proxy, fallback TLS
librefang-testing           Infrastruktura testowa: mockowany kernel, mockowany driver LLM i narzędzia do testowania tras API
librefang-telemetry         Instrumentacja metryk OpenTelemetry + Prometheus dla LibreFang
librefang-llm-driver        Trait drivera LLM i współdzielone typy dla LibreFang
librefang-llm-drivers       Konkretne drivery dostawców LLM (anthropic, openai, gemini, …) implementujące trait librefang-llm-driver
librefang-runtime-mcp       Klient MCP (Model Context Protocol) dla środowiska uruchomieniowego (runtime) LibreFang
librefang-kernel-handle     Trait KernelHandle dla wywołań wewnątrzprocesowych (in-process) do kernela LibreFang
librefang-runtime-wasm      Sandbox umiejętności (skills) WASM dla środowiska uruchomieniowego LibreFang
librefang-kernel-router     Silnik routingu Hand/Template dla kernela LibreFang
librefang-runtime-oauth     Przepływy OAuth (ChatGPT, GitHub Copilot) dla driverów środowiska uruchomieniowego LibreFang
librefang-kernel-metering   Pomiar kosztów i egzekwowanie limitów (quota) dla kernela LibreFang
xtask                       Automatyzacja budowania
```
> **OFP wire to plaintext-by-design.** Wzajemne uwierzytelnianie HMAC-SHA256 + HMAC
> dla każdej wiadomości + ochrona przed atakami typu replay (nonce) zabezpieczają przed *aktywnymi* atakującymi, ale zawartość ramek
> nie jest szyfrowana. W przypadku cross-network federation, uruchamiaj OFP za prywatnym
> overlayem (WireGuard, Tailscale, tunel SSH) lub warstwą mTLS typu service-mesh.
> Szczegóły: [docs.librefang.ai/architecture/ofp-wire](https://docs.librefang.ai/architecture/ofp-wire)

## Kluczowe Funkcje

**45 Adapterów Kanałów** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky i 32 innych. [Pełna lista](https://docs.librefang.ai/integrations/channels)

**28 Dostawców LLM** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama i 20 innych. Inteligentny routing, automatyczny fallback, śledzenie kosztów. [Szczegóły](https://docs.librefang.ai/configuration/providers)

**16 Warstw Zabezpieczeń** — Piaskownica (sandbox) WASM, ścieżka audytu Merkle, śledzenie skażenia (taint tracking), podpisywanie Ed25519, ochrona przed SSRF, zerowanie sekretów (secret zeroization) i wiele więcej. [Szczegóły](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth)

**API Kompatybilne z OpenAI** — Bezpośrednio podmienialny (drop-in) endpoint `/v1/chat/completions`. Ponad 140 endpointów REST/WS/SSE. [Dokumentacja API](https://docs.librefang.ai/integrations/api)

**Klienckie SDK** — Pełny klient REST ze wsparciem dla streamingu.

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

**Wsparcie dla MCP** — Wbudowany klient i serwer MCP. Połącz z IDE, rozszerzaj za pomocą niestandardowych narzędzi, twórz potoki (pipelines) agentów. [Szczegóły](https://docs.librefang.ai/integrations/mcp-a2a)

**Protokół A2A** — Wsparcie dla protokołu Google Agent-to-Agent. Odkrywaj, komunikuj się i deleguj zadania między systemami agentowymi. [Szczegóły](https://docs.librefang.ai/integrations/mcp-a2a)

**Aplikacja Desktopowa** — Natywna aplikacja Tauri 2.0 z zasobnikiem systemowym (system tray), powiadomieniami i globalnymi skrótami klawiszowymi.

**Migracja z OpenClaw** — Polecenie `librefang migrate --from openclaw` importuje agentów, historię, umiejętności i konfigurację.

## Development

```bash
cargo build --workspace --lib                            # Buduj
cargo test --workspace                                   # Ponad 2100 testów
cargo clippy --workspace --all-targets -- -D warnings    # Zero ostrzeżeń
cargo fmt --all -- --check                               # Sprawdzanie formatowania
```

## Porównanie

Zobacz [Porównanie](https://docs.librefang.ai/getting-started/comparison#16-security-systems--defense-in-depth), aby zapoznać się z benchmarkami i szczegółowym porównaniem funkcji w zestawieniu z OpenClaw, ZeroClaw, CrewAI, AutoGen oraz LangGraph.

## Linki

- [Dokumentacja](https://docs.librefang.ai) &bull; [Dokumentacja API](https://docs.librefang.ai/integrations/api) &bull; [Wprowadzenie](https://docs.librefang.ai/getting-started) &bull; [Rozwiązywanie problemów](https://docs.librefang.ai/operations/troubleshooting)
- [Współtworzenie](../CONTRIBUTING.md) &bull; [Zarządzanie](../GOVERNANCE.md) &bull; [Bezpieczeństwo](../SECURITY.md)
- Dyskusje: [Pytania i odpowiedzi (Q&A)](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [Przypadki użycia](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [Głosowanie na funkcje](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [Ogłoszenia](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## Współtwórcy

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../web/public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  Mile widziane są wszelkiego rodzaju kontrybucje — kod, dokumentacja, tłumaczenia, zgłoszenia błędów.<br/>
  Sprawdź <a href="../CONTRIBUTING.md">Przewodnik współtworzenia</a> i wybierz <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a>, aby zacząć!<br/>
  Możesz również odwiedzić <a href="https://leszek3737.github.io/librefang-WIki/">nieoficjalną wiki</a>, która jest aktualizowana o przydatne informacje dla nowych współtwórców.
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../web/public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">Licencja MIT</p>
