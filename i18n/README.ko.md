<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">자유로운 에이전트 운영체제 — Libre는 자유를 의미합니다</h3>

<p align="center">
  Rust로 구축된 오픈소스 Agent OS. 14개 크레이트. 2,100+ 테스트. clippy 경고 제로.
</p>

<p align="center">
  <a href="README.md">English</a> | <a href="i18n/README.zh.md">中文</a> | <a href="i18n/README.ja.md">日本語</a> | <a href="i18n/README.ko.md">한국어</a> | <a href="i18n/README.es.md">Español</a> | <a href="i18n/README.de.md">Deutsch</a> | <a href="i18n/README.pl.md">Polski</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">웹사이트</a> &bull;
  <a href="https://docs.librefang.ai">문서</a> &bull;
  <a href="../docs/CONTRIBUTING.md">기여</a> &bull;
  <a href="https://discord.gg/DzTYqAZZmc">Discord</a>
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/librefang/librefang/ci.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/v/release/librefang/librefang?style=flat-square" alt="Latest Release" />
  <a href="https://discord.gg/DzTYqAZZmc"><img src="https://img.shields.io/discord/1481633471507071129?style=flat-square&logo=discord&label=Discord" alt="Discord" /></a>
</p>

---

## LibreFang이란?

LibreFang은 **에이전트 운영체제**입니다 — Rust로 처음부터 구축된 자율형 AI 에이전트 실행 플랫폼입니다. 챗봇 프레임워크도, Python 래퍼도 아닙니다.

기존 에이전트 프레임워크는 입력을 기다립니다. LibreFang은 **당신을 위해 일하는 에이전트**를 실행합니다 — 스케줄에 따라 24/7, 타겟 모니터링, 리드 생성, 소셜 미디어 관리, 대시보드 보고를 수행합니다.

> LibreFang은 [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang)의 커뮤니티 포크로, 오픈 거버넌스와 머지 우선 PR 정책을 채택합니다. 자세한 내용은 [GOVERNANCE.md](../docs/GOVERNANCE.md)를 참조하세요.

<p align="center">
  <img src="../public/assets/dashboard.jpg" width="800" alt="LibreFang 대시보드" />
</p>

## 빠른 시작

```bash
# 설치 (Linux/macOS/WSL)
curl -fsSL https://librefang.ai/install.sh | sh

# 또는 Cargo로 설치
cargo install --git https://github.com/librefang/librefang librefang-cli

# 초기화 (프로바이더 설정 안내)
librefang init

# 시작 — 대시보드 http://localhost:4545
librefang start
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
<summary><strong>클라우드 배포</strong></summary>

[![Deploy Hub](https://img.shields.io/badge/Deploy%20Hub-000?style=for-the-badge&logo=rocket)](https://deploy.librefang.ai) [![Fly.io](https://img.shields.io/badge/Fly.io-purple?style=for-the-badge&logo=fly.io)](https://deploy.librefang.ai) [![Render](https://img.shields.io/badge/Render-46E3B7?style=for-the-badge&logo=render)](https://render.com/deploy?repo=https://github.com/librefang/librefang) [![Railway](https://img.shields.io/badge/Railway-0B0D0E?style=for-the-badge&logo=railway)](https://railway.app/template/librefang) [![GCP](https://img.shields.io/badge/GCP-4285F4?style=for-the-badge&logo=googlecloud)](../deploy/gcp/README.md)

</details>

## Hands: 당신을 위해 일하는 에이전트

**Hands**는 사전 구축된 자율형 기능 패키지로, 스케줄에 따라 독립적으로 작동합니다. 14개 내장:

| Hand | 기능 |
|------|------|
| **Researcher** | 심층 조사 — 다중 소스 교차 참조, CRAAP 신뢰도 평가, 인용 보고서 |
| **Collector** | OSINT 모니터링 — 변경 감지, 감정 추적, 지식 그래프 |
| **Predictor** | 초예측 — 신뢰 구간이 포함된 교정된 예측 |
| **Strategist** | 전략 분석 — 시장 조사, 경쟁 인텔리전스, 사업 계획 |
| **Analytics** | 데이터 분석 — 수집, 분석, 시각화, 자동 보고 |
| **Trader** | 시장 인텔리전스 — 다중 신호 분석, 리스크 관리, 포트폴리오 분석 |
| **Lead** | 잠재 고객 발굴 — 웹 조사, 스코어링, 중복 제거, 리드 전달 |
| **Twitter** | 자율형 X/Twitter — 콘텐츠 생성, 스케줄링, 승인 큐 |
| **Reddit** | Reddit 관리 — 서브레딧 모니터링, 게시, 참여 추적 |
| **LinkedIn** | LinkedIn 관리 — 콘텐츠 생성, 네트워킹, 전문적 교류 |
| **Clip** | YouTube 쇼츠 변환 — 베스트 모먼트 컷, 자막, AI 내레이션 |
| **Browser** | 웹 자동화 — Playwright 기반, 구매 승인 게이트 필수 |
| **API Tester** | API 테스트 — 엔드포인트 발견, 검증, 부하 테스트, 회귀 감지 |
| **DevOps** | DevOps 자동화 — CI/CD, 인프라 모니터링, 인시던트 대응 |

```bash
librefang hand activate researcher   # 즉시 작업 시작
librefang hand status researcher     # 진행 상황 확인
librefang hand list                  # 모든 Hands 보기
```

나만의 Hand 만들기: `HAND.toml` + 시스템 프롬프트 + `SKILL.md`를 정의하세요. [가이드](../docs/skill-development.md)

## 아키텍처

14개 Rust 크레이트, 모듈러 커널 설계.

```
librefang-kernel      오케스트레이션, 워크플로, 미터링, RBAC, 스케줄러, 예산
librefang-runtime     에이전트 루프, 3 LLM 드라이버, 53 도구, WASM 샌드박스, MCP, A2A
librefang-api         140+ REST/WS/SSE 엔드포인트, OpenAI 호환 API, 대시보드
librefang-channels    40 메시징 어댑터, 레이트 리미팅, DM/그룹 정책
librefang-memory      SQLite 영속화, 벡터 임베딩, 세션, 압축
librefang-types       코어 타입, 테인트 추적, Ed25519 서명, 모델 카탈로그
librefang-skills      60 번들 스킬, SKILL.md 파서, FangHub 마켓플레이스
librefang-hands       14 자율 Hands, HAND.toml 파서, 라이프사이클 관리
librefang-extensions  25 MCP 템플릿, AES-256-GCM 볼트, OAuth2 PKCE
librefang-wire        OFP P2P 프로토콜, HMAC-SHA256 상호 인증
librefang-cli         CLI, 데몬 관리, TUI 대시보드, MCP 서버 모드
librefang-desktop     Tauri 2.0 네이티브 앱 (트레이, 알림, 단축키)
librefang-migrate     OpenClaw, LangChain, AutoGPT 마이그레이션 엔진
xtask                 빌드 자동화
```

## 주요 기능

**40 채널 어댑터** — Telegram, Discord, Slack, WhatsApp, Signal, Matrix, Email, Teams, Google Chat, Feishu, LINE, Mastodon, Bluesky 등. [전체 목록](../docs/channel-adapters.md)

**27 LLM 프로바이더** — Anthropic, Gemini, OpenAI, Groq, DeepSeek, OpenRouter, Ollama 등. 지능형 라우팅, 자동 폴백, 비용 추적. [상세](../docs/providers.md)

**16 보안 레이어** — WASM 샌드박스, Merkle 감사 추적, 테인트 추적, Ed25519 서명, SSRF 보호, 시크릿 제로화 등. [상세](../docs/comparison.md#16-security-systems--defense-in-depth)

**OpenAI 호환 API** — 드롭인 `/v1/chat/completions` 엔드포인트. 140+ REST/WS/SSE 엔드포인트. [API 레퍼런스](../docs/api-reference.md)

**클라이언트 SDK** — 스트리밍 지원 완전한 REST 클라이언트.

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

**MCP 지원** — MCP 클라이언트 및 서버 내장. IDE 연동, 커스텀 도구 확장, 에이전트 파이프라인 구성. [상세](../docs/providers.md)

**A2A 프로토콜** — Google Agent-to-Agent 프로토콜 지원. 에이전트 시스템 간 탐색, 통신, 태스크 위임. [상세](../docs/api-reference.md)

**데스크톱 앱** — Tauri 2.0 네이티브 앱. 시스템 트레이, 알림, 글로벌 단축키.

**OpenClaw 마이그레이션** — `librefang migrate --from openclaw`로 에이전트, 히스토리, 스킬, 설정을 가져옵니다.

## 개발

```bash
cargo build --workspace --lib                            # 빌드
cargo test --workspace                                   # 2,100+ 테스트
cargo clippy --workspace --all-targets -- -D warnings    # 경고 제로
cargo fmt --all -- --check                               # 포맷 체크
```

## 비교

[docs/comparison.md](../docs/comparison.md)에서 OpenClaw, ZeroClaw, CrewAI, AutoGen, LangGraph와의 벤치마크 및 기능 비교를 확인하세요.

## 링크

- [문서](https://docs.librefang.ai) &bull; [API 레퍼런스](../docs/api-reference.md) &bull; [시작 가이드](../docs/getting-started.md) &bull; [문제 해결](../docs/troubleshooting.md)
- [기여](../docs/CONTRIBUTING.md) &bull; [거버넌스](../docs/GOVERNANCE.md) &bull; [보안](../docs/SECURITY.md)
- 토론: [Q&A](https://github.com/librefang/librefang/discussions/categories/q-a) &bull; [유스케이스](https://github.com/librefang/librefang/discussions/categories/show-and-tell) &bull; [기능 투표](https://github.com/librefang/librefang/discussions/categories/ideas) &bull; [공지](https://github.com/librefang/librefang/discussions/categories/announcements) &bull; [Discord](https://discord.gg/DzTYqAZZmc)

## 기여자

<a href="https://github.com/librefang/librefang/graphs/contributors">
  <img src="../public/assets/contributors.svg" alt="Contributors" />
</a>

<p align="center">
  코드, 문서, 번역, 버그 리포트 등 모든 형태의 기여를 환영합니다.<br/>
  <a href="../docs/CONTRIBUTING.md">기여 가이드</a>를 확인하고 <a href="https://github.com/librefang/librefang/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22">good first issue</a>부터 시작해 보세요!
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

<p align="center">MIT 라이선스</p>
