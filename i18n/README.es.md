<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">Sistema Operativo de Agentes Libre — Libre como en Libertad</h3>

<p align="center">
  Sistema operativo de agente de código abierto escrito en Rust. 137K líneas de código. 14 crates. 2,100+ pruebas. Cero advertencias de clippy.<br/>
  <strong>Bifurcado de <a href="https://github.com/RightNow-AI/openfang">RightNow-AI/openfang</a>. Gobernanza verdaderamente abierta. Contribuidores bienvenidos. Los PRs que ayudan al proyecto se fusionan.</strong>
</p>

<p align="center">
  <strong>Versiones en otros idiomas:</strong> <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">Sitio web</a> &bull;
  <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
  <a href="../GOVERNANCE.md">Gobernanza</a> &bull;
  <a href="../CONTRIBUTING.md">Contribuciones</a> &bull;
  <a href="../SECURITY.md">Seguridad</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/community-maintained-brightgreen?style=flat-square" alt="Mantenido por la comunidad" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/forks/librefang/librefang?style=flat-square" alt="Forks" />
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

> **LibreFang es una bifurcación comunitaria de [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang).**
>
> **"Libre"** significa libertad. Elegimos este nombre porque creemos que un proyecto de código abierto debe ser verdaderamente abierto — no solo en licencia, sino en gobernanza, contribución y colaboración. LibreFang toma un camino fundamentalmente diferente al proyecto upstream: damos la bienvenida a cada contribuidor, revisamos cada PR públicamente y fusionamos el trabajo que beneficia al proyecto.

> **Nuestra promesa a los contribuidores:**
> - Si tu PR ayuda positivamente al proyecto, **lo fusionamos tal cual** con atribución completa.
> - Si tu PR necesita mejoras, **lo revisamos activamente y proporcionamos sugerencias concretas** para ayudarte a fusionarlo — no cerramos PRs en silencio.
> - Cada contribuidor es valorado. Correcciones de bugs, documentación, pruebas, funcionalidades, empaquetado, traducciones — todas las contribuciones importan.

---

## ¿Por qué LibreFang? — La diferencia con OpenFang

LibreFang se bifurcó de [RightNow-AI/openfang](https://github.com/RightNow-AI/openfang) porque creemos en una forma diferente de gestionar un proyecto de código abierto.

### Lo que significa "Libre"

| | OpenFang | LibreFang |
|---|---------|-----------|
| **Licencia** | MIT + Apache-2.0 | MIT |
| **Gobernanza** | Controlada por una empresa | Gobernanza comunitaria, toma de decisiones transparente |
| **Política de PRs** | A discreción del mantenedor | Contribuciones positivas fusionadas tal cual; las demás reciben revisión activa con sugerencias de mejora |
| **Atribución** | Sin garantía | Siempre preservada en commits y notas de lanzamiento |
| **Contribuidores** | Participación limitada | Activamente bienvenidos — te necesitamos |
| **SLA de revisión** | Sin compromiso | Respuesta inicial en 7 días |

### Nuestros compromisos

- **Mentalidad de fusionar primero.** Si tu PR ayuda al proyecto a avanzar, lo fusionamos. Sin filtros, sin "lo reescribiremos internamente".
- **Revisión de código activa.** Los PRs que necesitan cambios reciben retroalimentación detallada y constructiva — no silencio. Te ayudamos a entregar.
- **Atribución completa.** Si un mantenedor adapta tu parche, tu nombre permanece en los metadatos del commit (`Co-authored-by`) y en las notas de lanzamiento. Cerrar un PR y reimplementarlo en privado está explícitamente prohibido por nuestra [gobernanza](../GOVERNANCE.md).
- **Gobernanza abierta.** Las decisiones técnicas ocurren en issues y PRs, no a puerta cerrada. Ver [`GOVERNANCE.md`](../GOVERNANCE.md) y [`MAINTAINERS.md`](../MAINTAINERS.md).
- **Únete a nosotros.** Los contribuidores activos son invitados a unirse a la org de LibreFang en GitHub. Los participantes principales que contribuyen consistentemente obtienen acceso de commit y voz en la dirección del proyecto.

---

## ¿Qué es LibreFang?

LibreFang es un **sistema operativo de agente de código abierto** — no es un marco de chatbot, no es un envoltorio de Python alrededor de LLM, no es un "orquestador multiagente". Es un sistema operativo completo para agentes autónomos, construido desde cero en Rust y mantenido públicamente.

Los marcos de agentes tradicionales esperan tu entrada. LibreFang ejecuta **agentes autónomos que trabajan para ti** — se ejecutan según horario, 24/7, construyen grafos de conocimiento, monitorean objetivos, generan leads, gestionan tus redes sociales y reportan resultados a tu panel.

El sitio web del proyecto ya está en vivo en [librefang.ai](https://librefang.ai/). La forma más rápida de probar LibreFang sigue siendo la instalación desde el código fuente.

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
librefang init
librefang start
# Panel: http://localhost:4545
```

**O instala con Homebrew:**
```bash
brew tap librefang/tap
brew install librefang
```

<a id="docker"></a>

**O ejecuta con Docker:**
```bash
docker run -p 4545:4545 ghcr.io/librefang/librefang
# Panel: http://localhost:4545
```

---

## Características Principales

### 🤖 Hands: Agentes que realmente hacen el trabajo

*"Los agentes tradicionales esperan tu entrada. Hands trabaja para ti."*

**Hands** es la innovación central de LibreFang — paquetes de capacidades autónomas pre-construidos, ejecutados independientemente, según horario, sin necesidad de que les des prompts. Esto no es un chatbot. Es un agente que se despierta a las 6 de la mañana, investiga a tus competidores, construye grafos de conocimiento, evalúa descubrimientos y te envía un informe a Telegram antes de que tomes tu café.

Cada Hand incluye:
- **HAND.toml** — Manifiesto que declara herramientas, requisitos y métricas del panel
- **System Prompt** — Manual de operaciones multietapa (no es una sola línea — son procedimientos de más de 500 palabras de expertos)
- **SKILL.md** — Referencia de conocimiento de dominio que se inyecta en el contexto en tiempo de ejecución
- **Guardrails** — Puertas de aprobación para operaciones sensibles (ej. Browser Hand necesita aprobación antes de cualquier compra)

Todo se compila en binarios. Sin descargas, sin pip install, sin Docker pull.

### 7 Hands Incluidos

| Hand | Funcionalidad |
|------|------|
| **Clip** | Obtiene URL de YouTube, descarga, identifica el mejor momento, recorta a video corto vertical con subtítulos y miniatura, opcionalmente agrega narración IA, publica en Telegram y WhatsApp. Pipeline de 8 etapas. FFmpeg + yt-dlp + 5 backends STT. |
| **Lead** | Se ejecuta diariamente. Descubre prospectos que coinciden con tu ICP, enriquece con investigación web, puntúa 0-100, deduplica con base de datos existente, entrega leads calificados en CSV/JSON/Markdown. Construye perfil ICP con el tiempo. |
| **Collector** | Inteligencia de nivel OSINT. Das un objetivo (empresa, persona, tema). Monitorea continuamente — detección de cambios, seguimiento de sentimiento, construcción de grafo de conocimiento, entrega alertas críticas cuando hay cambios importantes. |
| **Predictor** | Motor de superpronóstico. Recopila señales de múltiples fuentes, construye cadenas de inferencia calibradas, hace predicciones con intervalos de confianza, rastrea su propia precisión con puntuación Brier. Tiene modo adversario — deliberadamente discrepa del consenso. |
| **Researcher** | Investigador autónomo profundo. Cruza múltiples fuentes, evalúa credibilidad usando criterios CRAAP (Moneda, Relevancia, Autoridad, Exactitud, Propósito), genera informes en formato APA con citas, multilingüe. |
| **Twitter** | Gestor autónomo de cuentas Twitter/X. Crea contenido en 7 formatos rotativos, programa publicaciones para máximo engagement, responde a menciones, rastrea métricas de rendimiento. Tiene cola de aprobación — no publica sin tu OK. |
| **Browser** | Agente de automatización web. Navega sitios, llena formularios, hace clic en botones, maneja flujos de trabajo de múltiples pasos. Usa puente Playwright y persistencia de sesión. **Puerta de aprobación de compra forzada** — nunca gastará tu dinero sin confirmación explícita. |

---

## Sistema de Seguridad de 16 Capas — Defensa en Profundidad

LibreFang no añade seguridad como afterthought. Cada capa es independientemente testeable y funciona sin puntos únicos de falla.

| # | Sistema | Funcionalidad |
|---|---------|------|
| 1 | **Sandbox WASM de doble medición** | El código de herramientas se ejecuta en WebAssembly con medición de combustible + interrupción de época. Hilos watchdog matan código descontrolado. |
| 2 | **Cadena de hash Merkle de auditoría** | Cada operación se vincula criptográficamente con la anterior. Manipular una entrada rompe toda la cadena. |
| 3 | **Rastreo de tinte de flujo de información** | Las etiquetas se propagan durante la ejecución — rastrea secrets desde la fuente hasta el sumidero. |
| 4 | **Manifiesto de agente firmado Ed25519** | La identidad y conjunto de capacidades de cada agente están firmados criptográficamente. |
| 5 | **Protección SSRF** | Bloquea IPs privadas, endpoints de metadatos en la nube, ataques de DNS rebinding. |
| 6 | **Ceroización de secrets** | `Zeroizing<String>` borra claves API de la memoria inmediatamente cuando ya no son necesarias. |
| 7 | **Autenticación mutua OFP** | HMAC-SHA256 basado en nonce, verificación de tiempo constante para redes P2P. |
| 8 | **Puertas de capacidades** | Control de acceso basado en roles — los agentes declaran las herramientas que necesitan, el kernel las强制执行. |
| 9 | **Encabezados de seguridad** | CSP, X-Frame-Options, HSTS, X-Content-Type-Options en cada respuesta. |
| 10 | **Saneamiento de endpoint de salud** | Los health checks públicos devuelven información mínima. Diagnóstico completo requiere autenticación. |
| 11 | **Sandbox de subprocesos** | `env_clear()` + paso selectivo de variables. Aislamiento de árbol de procesos con kill multiplataforma. |
| 12 | **Escáner de inyección de prompts** | Detenta intentos de override, patrones de exfiltración, inyección de referencias de shell en skills. |
| 13 | **Guardia de bucles** | Detección de bucles de llamadas de herramientas basada en SHA256 con circuit breaker. Maneja patrones ping-pong. |
| 14 | **Reparación de sesión** | Validación de historial de mensajes de 7 etapas y recuperación automática de corrupción. |
| 15 | **Prevención de recorrido de rutas** | Normalización y prevención de escape de enlaces simbólicos. `../` no funciona aquí. |
| 16 | **Limitador de tasa GCRA** | Limitación de tasa de token bucket con conocimiento de costos, seguimiento por IP y limpieza de antiguo. |

---

## Arquitectura

14 crates de Rust. 137,728 líneas de código. Diseño de kernel modular.

```
librefang-kernel      Orquestación, flujos, medición, RBAC, programador, seguimiento de presupuesto
librefang-runtime     Bucle de agente, 3 drivers LLM, 53 herramientas, sandbox WASM, MCP, A2A
librefang-api         140+ endpoints REST/WS/SSE, API compatible con OpenAI, panel
librefang-channels    40 adaptadores de mensajes, con limitadores de tasa
librefang-memory      Persistencia SQLite, embeddings vectoriales, sesiones canónicas, compactación
librefang-types      Tipos centrales, rastreo de tinte, firma de manifiestos Ed25519, catálogo de modelos
librefang-skills     60 skills incluidos, parser de SKILL.md, mercado FangHub
librefang-hands      7 Hands autónomos, parser de HAND.toml, gestión de ciclo de vida
librefang-extensions 25 plantillas MCP, bóveda de credenciales AES-256-GCM, OAuth2 PKCE
librefang-wire       Protocolo P2P OFP, con autenticación mutua HMAC-SHA256
librefang-cli        CLI, gestión de daemon, panel TUI, modo servidor MCP
librefang-desktop    App nativa Tauri 2.0 (bandeja del sistema, notificaciones, atajos globales)
librefang-migrate    Motor de migración de OpenClaw, LangChain, AutoGPT
xtask                Automatización de construcción
```

---

## Inicio Rápido

```bash
# 1. Instalar
cargo install --git https://github.com/librefang/librefang librefang-cli

# 2. Inicializar — te guía a través de la configuración del proveedor
librefang init

# 3. Iniciar daemon
librefang start

# 4. Panel: http://localhost:4545

# 5. Activar una Hand — comienza a trabajar para ti
librefang hand activate researcher

# 6. Chatear con el agente
librefang chat researcher
> "¿Cuáles son las últimas tendencias en marcos de agentes de IA?"

# 7. Generar un agente preconstruido
librefang agent spawn coder
```

---

## Desarrollo

```bash
# Construir workspace
cargo build --workspace --lib

# Ejecutar todas las pruebas (2,100+)
cargo test --workspace

# Lint (debe ser 0 advertencias)
cargo clippy --workspace --all-targets -- -D warnings

# Formatear
cargo fmt --all -- --check
```

---

## Nota de Estabilidad

LibreFang es pre-1.0. La arquitectura es sólida, el conjunto de pruebas es completo, el modelo de seguridad es completo. Es decir:

- **Cambios rompedores** pueden ocurrir entre versiones menores hasta v1.0
- **Algunas Hands** son más maduras que otras (Browser y Researcher están más battle-tested)
- **Casos edge** existen — si encuentras uno, [abre un issue](https://github.com/librefang/librefang/issues)
- En producción **haz pin a un commit específico** hasta v1.0

Publicamos rápido, corregimos rápido. Objetivo: lanzar un v1.0 sólido a mediados de 2026.

---

## Seguridad

Para reportar vulnerabilidades de seguridad, sigue el proceso de reporte privado en [SECURITY.md](../SECURITY.md).

---

## Licencia

Licencia MIT. Ver archivo LICENSE.

---

## Enlaces

- [GitHub](https://github.com/librefang/librefang)
- [Sitio web](https://librefang.ai/)
- [Documentación](https://docs.librefang.ai)
- [Guía de contribuciones](../CONTRIBUTING.md)
- [Gobernanza](../GOVERNANCE.md)
- [Mantenedores](../MAINTAINERS.md)
- [Política de seguridad](../SECURITY.md)
- [Discusiones](https://github.com/librefang/librefang/discussions) — Preguntas, ideas y demos
- [Discord](https://discord.gg/DzTYqAZZmc) — Chat en tiempo real con la comunidad

---

## Contribuidores

¡Gracias a todos los que han contribuido a LibreFang!

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=librefang/librefang&max=100&columns=12" />
</a>

> ¿Quieres unirte? Consulta nuestra [Guía de contribuciones](../CONTRIBUTING.md) — muchas tareas no requieren experiencia en Rust.

---

<p align="center">
  <strong>Construido en Rust. 16 capas de seguridad. Agentes que realmente trabajan para ti.</strong>
</p>
