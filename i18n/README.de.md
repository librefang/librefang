<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Freies Agenten-Betriebssystem — Libre bedeutet Freiheit</h3>

<p align="center">
  Open-Source-Agent-OS in Rust geschrieben. 137K Codezeilen. 14 Crates. 2,100+ Tests. Keine Clippy-Warnungen.<br/>
  <strong>Geforkt von <a href="https://github.com/RightNow-AI/openfang">RightNow-AI/openfang</a>. Wirklich offene Governance. Mitwirkende willkommen. PRs, die dem Projekt helfen, werden gemergt.</strong>
</p>

<p align="center">
  <strong>Sprachversionen:</strong> <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Website</a> &bull;
  <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
  <a href="../GOVERNANCE.md">Governance</a> &bull;
  <a href="../CONTRIBUTING.md">Beiträge</a> &bull;
  <a href="../SECURITY.md">Sicherheit</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/community-maintained-brightgreen?style=flat-square" alt="Community-gepflegt" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/forks/librefang/librefang?style=flat-square" alt="Forks" />
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

> **LibreFang ist ein Community-Fork von [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang).**
>
> **"Libre"** bedeutet Freiheit. Wir haben diesen Namen gewählt, weil wir glauben, dass ein Open-Source-Projekt wirklich offen sein sollte — nicht nur in der Lizenz, sondern auch in Governance, Beitrag und Zusammenarbeit. LibreFang geht einen grundlegend anderen Weg als das Upstream-Projekt: Wir heißen jeden Mitwirkenden willkommen, überprüfen jeden PR öffentlich und mergen Arbeit, die dem Projekt nützt.

> **Unser Versprechen an Mitwirkende:**
> - Wenn dein PR dem Projekt positiv hilft, **mergen wir ihn unverändert** mit vollständiger Zuschreibung.
> - Wenn dein PR Verbesserungen braucht, **reviewen wir ihn aktiv und geben konkrete Verbesserungsvorschläge** um dir beim Mergen zu helfen — wir schließen PRs nicht stillschweigend.
> - Jeder Mitwirkende wird geschätzt. Bugfixes, Dokumentation, Tests, Features, Paketierung, Übersetzungen — alle Beiträge zählen.

---

## Warum LibreFang? — Der Unterschied zu OpenFang

LibreFang wurde von [RightNow-AI/openfang](https://github.com/RightNow-AI/openfang) geforkt, weil wir an eine andere Art glauben, ein Open-Source-Projekt zu führen.

### Was "Libre" bedeutet

| | OpenFang | LibreFang |
|---|---------|-----------|
| **Lizenz** | MIT + Apache-2.0 | MIT |
| **Governance** | Von einem Unternehmen kontrolliert | Community-Governance, transparente Entscheidungsfindung |
| **PR-Richtlinie** | Nach Ermessen des Maintainers | Positive Beiträge werden unverändert gemergt; andere erhalten aktives Review mit Verbesserungsvorschlägen |
| **Zuschreibung** | Nicht garantiert | Immer in Commits und Release-Notes beibehalten |
| **Mitwirkende** | Begrenzte Beteiligung | Aktiv willkommen — wir brauchen dich |
| **Review-SLA** | Keine Zusage | Erste Antwort innerhalb von 7 Tagen |

### Unsere Verpflichtungen

- **Merge-First-Mentalität.** Wenn dein PR dem Projekt hilft voranzukommen, mergen wir ihn. Kein Gatekeeping, kein "wir schreiben es intern um".
- **Aktives Code-Review.** PRs, die Änderungen brauchen, erhalten detailliertes, konstruktives Feedback — keine Stille. Wir helfen dir beim Ausliefern.
- **Vollständige Zuschreibung.** Wenn ein Maintainer deinen Patch anpasst, bleibt dein Name in den Commit-Metadaten (`Co-authored-by`) und Release-Notes. Einen PR zu schließen und privat neu zu implementieren ist durch unsere [Governance](../GOVERNANCE.md) ausdrücklich verboten.
- **Offene Governance.** Technische Entscheidungen finden in Issues und PRs statt, nicht hinter verschlossenen Türen. Siehe [`GOVERNANCE.md`](../GOVERNANCE.md) und [`MAINTAINERS.md`](../MAINTAINERS.md).
- **Mach mit.** Aktive Mitwirkende werden eingeladen, der LibreFang GitHub-Organisation beizutreten. Kernbeteiligte, die kontinuierlich beitragen, erhalten Commit-Zugang und Mitspracherecht bei der Projektrichtung.

---

## Was ist LibreFang?

LibreFang ist ein **Open-Source-Agent-Betriebssystem** — kein Chatbot-Framework, kein Python-Wrapper um LLMs, kein "Multi-Agent-Orchestrator". Es ist ein vollständiges Betriebssystem für autonome Agenten, von Grund auf in Rust aufgebaut und öffentlich gepflegt.

Traditionelle Agenten-Frameworks warten auf deine Eingabe. LibreFang führt **autonome Agenten aus, die für dich arbeiten** — laufen nach Zeitplan, rund um die Uhr, bauen Wissensgraphen auf, überwachen Ziele, generieren Leads, verwalten deine sozialen Medien und berichten Ergebnisse an dein Dashboard.

Die Projekt-Website ist jetzt unter [librefang.ai](https://librefang.ai/) live. Der schnellste Weg, LibreFang auszuprobieren, ist immer noch die Installation aus dem Quellcode.

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
librefang init
librefang start
# Dashboard: http://localhost:4545
```

**Oder mit Homebrew installieren:**
```bash
brew tap librefang/tap
brew install librefang
```

**Oder mit Docker ausführen:**
```bash
docker compose up --build
# Dashboard: http://localhost:4545
```

---

## Kernfunktionen

### 🤖 Hands: Agenten, die wirklich arbeiten

*"Traditionelle Agenten warten auf deine Eingabe. Hands arbeiten für dich."*

**Hands** ist die Kerninnovation von LibreFang — vorgefertigte autonome Fähigkeitspakete, die unabhängig laufen, nach Zeitplan, ohne dass du ihnen Prompts gibst. Das ist kein Chatbot. Das ist ein Agent, der um 6 Uhr morgens aufsteht, deine Konkurrenten erforscht, Wissensgraphen aufbaut, Erkennungen bewertet und dir einen Bericht auf Telegram schickt, bevor du deinen Kaffee trinkst.

Jedes Hand enthält:
- **HAND.toml** — Manifest, das Tools, Anforderungen und Dashboard-Kennzahlen deklariert
- **System Prompt** — Mehrstufiges Operationshandbuch (kein Einzeiler — das sind 500+ Wörter an Expertenverfahren)
- **SKILL.md** — Domänenexperten-Referenz, die zur Laufzeit in den Kontext injiziert wird
- **Guardrails** — Genehmigungstore für sensible Operationen (z.B. Browser Hand braucht vor jedem Kauf eine Genehmigung)

Alles wird in Binärdateien kompiliert. Kein Download, kein pip install, kein Docker pull.

### 7 gebündelte Hands

| Hand | Funktionalität |
|------|------|
| **Clip** | YouTube-URL holen, herunterladen, besten Moment identifizieren, zu kurzem vertikalem Video mit Untertiteln und Thumbnail schneiden, optional KI-Narration hinzufügen, auf Telegram und WhatsApp veröffentlichen. 8-Stufen-Pipeline. FFmpeg + yt-dlp + 5 STT-Backends. |
| **Lead** | Täglich. Findet zu deinem ICP passende Leads, bereichert durch Webrecherche, bewertet 0-100, dedupliziert mit bestehender Datenbank, liefert qualifizierte Leads in CSV/JSON/Markdown. Baut mit der Zeit ICP-Profil auf. |
| **Collector** | OSINT-Level-Intelligence. Gibt ein Ziel (Firma, Person, Thema). Überwacht kontinuierlich — Änderungserkennung, Sentiment-Tracking, Wissensgraph-Aufbau, liefert kritische Alerts bei wichtigen Änderungen. |
| **Predictor** | Superforecasting-Engine. Sammelt Signale aus mehreren Quellen, baut kalibrierte Inferenzketten auf, macht Vorhersagen mit Konfidenzintervallen, verfolgt eigene Genauigkeit mit Brier-Score. Hat Adversarial-Modus — widerspricht bewusst dem Konsens. |
| **Researcher** | Tiefgehender autonomer Forscher. Querverweist mehrere Quellen, bewertet Glaubwürdigkeit nach CRAAP-Kriterien (Währung, Relevanz, Autorität, Genauigkeit, Zweck), generiert APA-formatierte Berichte mit Zitaten, mehrsprachig. |
| **Twitter** | Autonomer Twitter/X-Konto-Manager. Erstellt Content in 7 Rotationsformaten, plant Posts für optimales Engagement, antwortet auf Erwähnungen, verfolgt Performance-Kennzahlen. Hat Genehmigungsqueue — nichts wird ohne dein OK gepostet. |
| **Browser** | Web-Automatisierungsagent. Navigiert Seiten, füllt Formulare aus, klickt Buttons, verarbeitet mehrstufige Workflows. Nutzt Playwright-Brücke und Session-Persistenz. **Erzwungenes Kauf-Genehmigungstor** — gibt nie dein Geld ohne explizite Bestätigung aus. |

---

## 16-Schichten-Sicherheitssystem — Verteidigung in der Tiefe

LibreFang fügt Sicherheit nicht nachträglich hinzu. Jede Schicht ist unabhängig testbar und läuft ohne Single Points of Failure.

| # | System | Funktionalität |
|---|---------|------|
| 1 | **WASM Dual-Metering-Sandbox** | Tool-Code läuft in WebAssembly mit Fuel-Metering + Epoch-Interrupt. Watchdog-Threads töten außer Kontrolle geratenen Code. |
| 2 | **Merkle-Hash-Chain Audit Trail** | Jede Operation ist kryptografisch mit der vorherigen verlinkt. Ein manipulierter Eintrag zerbricht die gesamte Kette. |
| 3 | **Information Flow Taint Tracking** | Labels propagieren während der Ausführung — verfolgt Secrets von der Quelle bis zur Senke. |
| 4 | **Ed25519 signierter Agent-Manifest** | Identität und Fähigkeiten jedes Agenten sind kryptografisch signiert. |
| 5 | **SSRF-Schutz** | Blockiert private IPs, Cloud-Metadaten-Endpunkte, DNS-Rebinding-Angriffe. |
| 6 | **Secret Zeroisierung** | `Zeroizing<String>` löscht API-Keys sofort aus dem Speicher, wenn nicht mehr benötigt. |
| 7 | **OFP Gegenseitige Authentifizierung** | HMAC-SHA256 nonce-basiert, constant-time Verifikation für P2P-Networking. |
| 8 | **Capability-Gates** | Rollenbasierte Zugriffskontrolle — Agenten deklarieren benötigte Tools, Kernel erzwingt sie. |
| 9 | **Sicherheitsheader** | CSP, X-Frame-Options, HSTS, X-Content-Type-Options auf jeder Antwort. |
| 10 | **Health Endpoint Sanitization** | Öffentliche Health-Checks geben minimalste Informationen zurück. Volle Diagnose erfordert Authentifizierung. |
| 11 | **Subprozess-Sandbox** | `env_clear()` + selektive Variable-Weiterleitung. Prozessbaum-Isolierung mit plattformübergreifendem kill. |
| 12 | **Prompt-Injection-Scanner** | Erkennt Override-Versuche, Exfiltrationsmuster, Shell-Referenz-Injection in Skills. |
| 13 | **Loop Guard** | SHA256-basierte Tool-Aufruf-Loop-Erkennung mit Circuit-Breaker. Handhabt Ping-Pong-Muster. |
| 14 | **Sitzungsreparatur** | 7-stufige Nachrichtenverlaufsvalidierung und automatische Wiederherstellung von Korruption. |
| 15 | **Pfad-Traversal-Prävention** | Normalisierung und Symlink-Escape-Prävention. `../` funktioniert hier nicht. |
| 16 | **GCRA-Rate-Limiter** | Kostenbewusster Token-Bucket-Rate-Limit mit per-IP-Tracking und Alt-Cleanup. |

---

## Architektur

14 Rust Crates. 137.728 Codezeilen. Modulares Kernel-Design.

```
librefang-kernel      Orchestrierung, Workflow, Metering, RBAC, Scheduler, Budget-Tracking
librefang-runtime     Agenten-Loop, 3 LLM-Treiber, 53 Tools, WASM-Sandbox, MCP, A2A
librefang-api         140+ REST/WS/SSE Endpoints, OpenAI-kompatibles API, Dashboard
librefang-channels    40 Nachrichtenadapter, mit Rate-Limiter
librefang-memory      SQLite-Persistenz, Vektor-Embeddings, Kanonische Sitzungen, Komprimierung
librefang-types       Kerntypen, Taint-Tracking, Ed25519 Manifest-Signatur, Modellkatalog
librefang-skills      60 gebündelte Skills, SKILL.md Parser, FangHub-Marktplatz
librefang-hands       7 autonome Hands, HAND.toml Parser, Lifecycle-Management
librefang-extensions  25 MCP-Vorlagen, AES-256-GCM Credential-Vault, OAuth2 PKCE
librefang-wire        OFP P2P-Protokoll, mit HMAC-SHA256 Gegenseitiger Authentifizierung
librefang-cli         CLI, Daemon-Management, TUI-Dashboard, MCP-Server-Modus
librefang-desktop     Tauri 2.0 native App (System-Tray, Benachrichtigungen, globale Shortcuts)
librefang-migrate     OpenClaw, LangChain, AutoGPT Migrations-Engine
xtask                Build-Automatisierung
```

---

## Schnellstart

```bash
# 1. Installieren
cargo install --git https://github.com/librefang/librefang librefang-cli

# 2. Initialisieren — führt dich durch die Provider-Einrichtung
librefang init

# 3. Daemon starten
librefang start

# 4. Dashboard: http://localhost:4545

# 5. Hand aktivieren — beginnt für dich zu arbeiten
librefang hand activate researcher

# 6. Mit dem Agenten chatten
librefang chat researcher
> "Was sind die neuesten Trends bei KI-Agenten-Frameworks?"

# 7. Einen vorgefertigten Agenten spawnen
librefang agent spawn coder
```

---

## Entwicklung

```bash
# Workspace bauen
cargo build --workspace --lib

# Alle Tests ausführen (2,100+)
cargo test --workspace

# Lint (muss 0 Warnungen sein)
cargo clippy --workspace --all-targets -- -D warnings

# Formatieren
cargo fmt --all -- --check
```

---

## Stabilitätshinweis

LibreFang ist pre-1.0. Die Architektur ist solide, die Test-Suite ist umfassend, das Sicherheitsmodell ist umfassend. Das heißt:

- **Breaking Changes** können zwischen Minor-Versionen bis v1.0 auftreten
- **Einige Hands** sind reifer als andere (Browser und Researcher sind am meisten battle-getestet)
- **Edge Cases** existieren — wenn du einen findest, [öffne ein Issue](https://github.com/librefang/librefang/issues)
- In Produktion **auf einen spezifischen Commit pinnen** bis v1.0

Wir veröffentlichen schnell, wir beheben schnell. Ziel: Mitte 2026 ein stabiles v1.0 veröffentlichen.

---

## Sicherheit

Um Sicherheitslücken zu melden, folge dem privaten Berichtsprozess in [SECURITY.md](../SECURITY.md).

---

## Lizenz

MIT-Lizenz. Siehe LICENSE-Datei.

---

## Links

- [GitHub](https://github.com/librefang/librefang)
- [Website](https://librefang.ai/)
- [Dokumentation](https://docs.librefang.ai)
- [Beitragsleitfaden](../CONTRIBUTING.md)
- [Governance](../GOVERNANCE.md)
- [Maintainer](../MAINTAINERS.md)
- [Sicherheitsrichtlinie](../SECURITY.md)
- [Diskussionen](https://github.com/librefang/librefang/discussions) — Fragen, Ideen und Vorstellungen

---

<p align="center">
  <strong>In Rust gebaut. 16 Schichten Sicherheit. Agenten, die wirklich für dich arbeiten.</strong>
</p>
