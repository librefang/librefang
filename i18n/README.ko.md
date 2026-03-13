<p align="center">
  <img src="../public/assets/logo.png" width="160" alt="LibreFang Logo" />
</p>

<h1 align="center">LibreFang</h1>
<h3 align="center">자유로운 Agent OS — Libre는 자유를 의미합니다</h3>

<p align="center">
  Rust로 작성된 오픈소스 Agent OS. 137K 코드 라인. 14개 crate. 1767+ 테스트. 경고 없음.<br/>
  <strong><a href="https://github.com/RightNow-AI/openfang">RightNow-AI/openfang</a>에서 포크. 진정한 오픈 거버넌스. 기여자 환영. 프로젝트에 도움이 되는 PR은 병합됩니다.</strong>
</p>

<p align="center">
  <strong>다국어 버전:</strong> <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | <a href="README.ja.md">日本語</a> | <a href="README.ko.md">한국어</a> | <a href="README.es.md">Español</a> | <a href="README.de.md">Deutsch</a>
</p>

<p align="center">
  <a href="https://librefang.ai/">웹사이트</a> &bull;
  <a href="https://github.com/librefang/librefang">GitHub</a> &bull;
  <a href="../GOVERNANCE.md">거버넌스</a> &bull;
  <a href="../CONTRIBUTING.md">기여</a> &bull;
  <a href="../SECURITY.md">보안</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/community-maintained-brightgreen?style=flat-square" alt="커뮤니티 관리" />
  <img src="https://img.shields.io/github/stars/librefang/librefang?style=flat-square" alt="Stars" />
  <img src="https://img.shields.io/github/forks/librefang/librefang?style=flat-square" alt="Forks" />
</p>

<p align="center">
  <a href="https://github.com/librefang/librefang/stargazers">
    <img src="../public/assets/star-history.svg" alt="Star History" />
  </a>
</p>

---

> **LibreFang은 [`RightNow-AI/openfang`](https://github.com/RightNow-AI/openfang)의 커뮤니티 포크입니다.**
>
> **"Libre"** 는 자유를 의미합니다. 오픈소스 프로젝트는 라이선스뿐만 아니라 거버넌스, 기여, 협업에서도 진정으로 개방적이어야 한다고 믿습니다. LibreFang은 업스트림 프로젝트와 근본적으로 다른 길을 걷고 있습니다: 모든 기여자를 환영하고, 모든 PR을 공개적으로 검토하며, 프로젝트에 도움이 되는 작업을 병합합니다.

> **기여자에 대한 약속:**
> - PR이 프로젝트에 긍정적인 도움이 되면, **전체 귀속을 유지하면서 그대로 병합**합니다.
> - PR에 개선이 필요하면, **적극적으로 리뷰하고 구체적인 개선 제안을 제공**하여 병합을 돕습니다 — PR을 조용히 닫지 않습니다.
> - 모든 기여자를 소중히 여깁니다. 버그 수정, 문서, 테스트, 기능, 패키징, 번역 — 모든 기여가 중요합니다.

---

## 왜 LibreFang인가? — OpenFang과의 차이

LibreFang은 오픈소스 프로젝트를 운영하는 다른 방식을 믿기 때문에 [RightNow-AI/openfang](https://github.com/RightNow-AI/openfang)에서 포크했습니다.

### "Libre"의 의미

| | OpenFang | LibreFang |
|---|---------|-----------|
| **라이선스** | MIT | MIT + Apache-2.0 |
| **거버넌스** | 단일 회사 통제 | 커뮤니티 거버넌스, 투명한 의사결정 |
| **PR 정책** | 메인테이너 재량 | 긍정적 기여는 그대로 병합; 나머지는 개선 제안과 함께 적극적 리뷰 |
| **귀속** | 보장 없음 | 커밋과 릴리스 노트에 항상 유지 |
| **기여자** | 제한적 참여 | 적극 환영 — 당신이 필요합니다 |
| **리뷰 SLA** | 약속 없음 | 7일 이내 첫 응답 |

### 우리의 약속

- **병합 우선.** PR이 프로젝트 발전에 도움이 되면, 병합합니다. 게이트키핑 없음, "내부에서 다시 작성" 없음.
- **적극적인 코드 리뷰.** 수정이 필요한 PR은 상세하고 건설적인 피드백을 받습니다 — 침묵이 아닌. 코드 출시를 돕습니다.
- **완전한 귀속 유지.** 메인테이너가 패치를 수정할 때, 커밋 메타데이터(`Co-authored-by`)와 릴리스 노트에 이름이 유지됩니다. PR을 닫고 비공개로 재구현하는 것은 [거버넌스](../GOVERNANCE.md)에서 명시적으로 금지됩니다.
- **오픈 거버넌스.** 기술적 결정은 이슈와 PR에서 공개적으로 이루어지며, 비공개가 아닙니다. [`GOVERNANCE.md`](../GOVERNANCE.md)와 [`MAINTAINERS.md`](../MAINTAINERS.md)를 참조하세요.
- **함께하세요.** 활발한 기여자는 LibreFang GitHub org에 초대됩니다. 지속적으로 기여하는 핵심 참여자에게는 커밋 권한과 프로젝트 방향에 대한 발언권이 주어집니다.

---

## LibreFang이란?

LibreFang은 **오픈소스 Agent 운영 체제**입니다 — 채팅봇 프레임워크가 아니고, LLM을 감싸는 Python도 아니고, "멀티 에이전트 오케스트레이터"도 아닙니다. Rust로 처음부터 구축된 자율형 에이전트를 위한 완전한 운영 체제입니다.

기존 에이전트 프레임워크는 당신의 입력을 기다립니다. LibreFang은 **당신을 위해 일하는 자율형 에이전트**를 실행합니다 — 일정에 따라, 24시간 동안, 지식 그래프를 구축하고, 대상을 모니터링하며, 리드를 생성하고, 소셜 미디어를 관리하고, 대시보드에 결과를 보고합니다.

프로젝트 웹사이트가 [librefang.ai](https://librefang.ai/)에 출시되었습니다. LibreFang을Trial하는 가장 빠른 방법은 여전히 소스からの 설치입니다.

```bash
cargo install --git https://github.com/librefang/librefang librefang-cli
librefang init
librefang start
# 대시보드: http://localhost:4545
```

**또는 Homebrew로 설치:**
```bash
brew tap librefang/tap
brew install librefang
```

---

## 핵심 기능

### 🤖 Hands: 실제로 작업을 수행하는 에이전트

*"기존 에이전트는 당신의 입력을 기다립니다. Hands는 당신을 위해 일합니다."*

**Hands**는 LibreFang의 핵심 혁신입니다 — 사전 구축된 자율형 능력 패키지로, 독립적으로 실행되고, 일정에 따라, 당신이 프롬프트를 입력하지 않고도 작동합니다. 채팅봇이 아닙니다. 오전 6시에 일어나서 경쟁사를 연구하고, 지식 그래프를 구축하고, 발견을 평가하고, 커피를 마시기 전에 Telegram으로 보고서를 보내는 에이전트입니다.

각 Hand는 다음을 포함합니다:
- **HAND.toml** — 도구, 요구 사항, 대시보드 지표를 선언하는 매니페스트
- **System Prompt** — 다단계 운영 매뉴얼 (한 줄이 아니라 500+ 단어의 전문가 절차)
- **SKILL.md** — 런타임에 컨텍스트에 주입되는 도메인 전문 지식 참조
- **Guardrails** — 민감한 작업에 대한 승인 게이트 (예: Browser Hand는 구매 전 승인이 필요)

모두 바이너리로 컴파일됩니다. 다운로드 불필요, pip install 불필요, Docker pull 불필요.

### 7개의 번들 Hands

| Hand | 기능 |
|------|------|
| **Clip** | YouTube URL 가져오기, 다운로드, 최고의 순간 식별, 자막과 썸네일이 포함된 짧은 세로 비디오로 자르기, 선택적 AI 내레이션 추가, Telegram 및 WhatsApp에 게시. 8단계 파이프라인. FFmpeg + yt-dlp + 5 STT 백엔드. |
| **Lead** | 매일 실행. ICP와 일치하는 잠재 고객 발견, 웹 리서치로 강화, 0-100점 점수 매기기, 기존 데이터베이스와 중복 제거, CSV/JSON/Markdown으로 적격 리드 제공. 시간이 지나면서 ICP 프로파일 구축. |
| **Collector** | OSINT 등급 인텔리전스. 대상 제공 (회사, 사람, 주제). 지속적으로 모니터링 — 변경 감지, 감정 추적, 지식 그래프 구축, 중요한 변화 시 중요한 알림 제공. |
| **Predictor** | 슈퍼포캐스팅 엔진. 여러 소스에서 시그널 수집, 보정된 추론 체인 구축, 신뢰 구간으로 예측, Brier 점수로 자체 정확성 추적. 반대 모드 있음 — 의도적으로 합의에 이의 제기. |
| **Researcher** | 심층 자율 연구자. 여러 소스 상호 참조, CRAAP 기준(통화, 관련성, 권위, 정확성, 목적)으로 신뢰성 평가, 인용이 포함된 APA 형식 보고서 생성, 다국어 지원. |
| **Twitter** | 자율 Twitter/X 계정 매니저. 7개 로테이션 형식으로 콘텐츠 생성, 최적의 참여를 위해 게시물 스케줄링, 멘션에 응답, 성능 지표 추적. 승인 대기열 있음 — 당신의 확인 없이는 게시하지 않음. |
| **Browser** | 웹 자동화 에이전트. 사이트 탐색, 양식 입력, 버튼 클릭, 다단계 워크플로 처리. Playwright 브릿지 및 세션 지속성 사용. **강제 구매 승인 게이트** — 명시적인 확인 없이는 당신의 돈을 사용하지 않음. |

---

## 16단계 보안 시스템 — 심층 방어

LibreFang은 사후에 보안을 추가하지 않습니다. 각 단계는 독립적으로 테스트 가능하며 단일 장애점 없이 실행됩니다.

| # | 시스템 | 기능 |
|---|---------|------|
| 1 | **WASM 2중 계량 샌드박스** | 도구 코드는 연료 계량 + 에포크 중단이 있는 WebAssembly에서 실행. 워치독 스레드가失控 코드를 종료. |
| 2 | **Merkle 해시 체인 감사 추적** | 각 작업은 암호학적으로 이전 것에 링크됨. 한 항목이라도 조작하면 전체 체인이 손상. |
| 3 | **정보 흐름 테인트 추적** | 레이블이 실행 중 전파 — 소스에서 싱크까지 secrets 추적. |
| 4 | **Ed25519 서명 에이전트 매니페스트** | 각 에이전트의 신원과 능력 세트가 암호학적으로 서명됨. |
| 5 | **SSRF 보호** | 개인 IP, 클라우드 메타데이터 엔드포인트, DNS rebinding 공격 차단. |
| 6 | **Secret 제로화** | `Zeroizing<String>`이 더 이상 필요하지 않을 때 즉시 메모리에서 API 키 삭제. |
| 7 | **OFP 상호 인증** | HMAC-SHA256 nonce 기반, P2P 네트워킹을 위한 상수 시간 검증. |
| 8 | **역할 기반 접근 제어** | 에이전트가 필요한 도구를 선언, 커널이 강제 실행. |
| 9 | **보안 헤더** | CSP, X-Frame-Options, HSTS, X-Content-Type-Options 모든 응답에 적용. |
| 10 | **헬스 엔드포인트 정비** | 공용 헬스 체크는 최소 정보 반환. 전체 진단에는 인증 필요. |
| 11 | **서브프로세스 샌드박스** | `env_clear()` + 선택적 변수 통과. 플랫폼 간 kill과 프로세스 트리 격리. |
| 12 | **프롬프트 주입 스캐너** | 오버라이드 시도, 데이터 추출 패턴, 스킬 내 셸 참조 주입 감지. |
| 13 | **루프 가드** | SHA256 기반 도구 호출 루프 감지 및 서킷 브레이커. ping-pong 패턴 처리. |
| 14 | **세션 복구** | 7단계 메시지 기록 검증 및 손상からの 자동 복구. |
| 15 | **경로 순회 방지** | 정규화 및 심볼릭 링크 탈출 방지. `../`는 여기서 작동하지 않음. |
| 16 | **GCRA 속도 제한기** | per-IP 추적 및 오래된 정리 기능이 있는 비용 인식 토큰 버킷 속도 제한. |

---

## 아키텍처

14개 Rust crate. 137,728줄 코드. 모듈식 커널 디자인.

```
librefang-kernel      오케스트레이션, 워크플로, 계량, RBAC, 스케줄러, 예산 추적
librefang-runtime     에이전트 루프, 3개 LLM 드라이버, 53개 도구, WASM 샌드박스, MCP, A2A
librefang-api         140+ REST/WS/SSE 엔드포인트, OpenAI 호환 API, 대시보드
librefang-channels    40개 메시지 어댑터, 속도 제한기 포함
librefang-memory      SQLite 지속성, 벡터 임베딩, 표준 세션, 컴팩션
librefang-types       핵심 타입, 테인트 추적, Ed25519 매니페스트 서명, 모델 카탈로그
librefang-skills      60개 번들 스킬, SKILL.md 파서, FangHub 마켓플레이스
librefang-hands      7개 자율 Hands, HAND.toml 파서, 라이프사이클 관리
librefang-extensions  25개 MCP 템플릿, AES-256-GCM 자격 증명 볼트, OAuth2 PKCE
librefang-wire        OFP P2P 프로토콜, HMAC-SHA256 상호 인증 포함
librefang-cli        CLI, 데몬 관리, TUI 대시보드, MCP 서버 모드
librefang-desktop    Tauri 2.0 네이티브 앱 (시스템 트레이, 알림, 전역 단축키)
librefang-migrate    OpenClaw, LangChain, AutoGPT 마이그레이션 엔진
xtask                빌드 자동화
```

---

## 빠른 시작

```bash
# 1. 설치
cargo install --git https://github.com/librefang/librefang librefang-cli

# 2. 초기화 — 공급자 설정 안내
librefang init

# 3. 데몬 시작
librefang start

# 4. 대시보드: http://localhost:4545

# 5. Hand 활성화 — 당신을 위해 일하기 시작
librefang hand activate researcher

# 6. 에이전트와 채팅
librefang chat researcher
> "AI 에이전트 프레임워크의 최신 동향은?"

# 7. 사전 구축 에이전트 스폰
librefang agent spawn coder
```

---

## 개발

```bash
# 워크스페이스 빌드
cargo build --workspace --lib

# 모든 테스트 실행 (1767+)
cargo test --workspace

# 린트 (경고 0개 필수)
cargo clippy --workspace --all-targets -- -D warnings

# 포맷
cargo fmt --all -- --check
```

---

## 안정성 참고

LibreFang은 pre-1.0입니다. 아키텍처는 견고하고, 테스트 스위트는 포괄적이며, 보안 모델도 포괄적입니다. 즉:

- **-breaking Changes**는 v1.0까지 마이너 버전 간에 발생할 수 있습니다
- **일부 Hands**는 다른 것보다 더 성숙합니다 (Browser와 Researcher가 가장 실전 경험이 많음)
- **엣지 케이스**가 존재합니다 — 발견하면 [이슈를 열어](https://github.com/librefang/librefang/issues)
- v1.0까지 프로덕션 배포에서는 **특정 커밋에 고정**하세요

우리는 빠른 출시, 빠른 수정합니다. 목표: 2026년 중반에 안정적인 v1.0을 출시합니다.

---

## 보안

보안 취약점을 보고하려면 [SECURITY.md](../SECURITY.md)의 비공개 보고 절차를 따르세요.

---

## 라이선스

MIT 라이선스. LICENSE 파일을 참조하세요.

---

## 링크

- [GitHub](https://github.com/librefang/librefang)
- [웹사이트](https://librefang.ai/)
- [문서](https://docs.librefang.ai)
- [기여 가이드](../CONTRIBUTING.md)
- [거버넌스](../GOVERNANCE.md)
- [메인테이너](../MAINTAINERS.md)
- [보안 정책](../SECURITY.md)

---

<p align="center">
  <strong>Rust로 구축. 16단계 보안. 실제로 당신을 위해 일하는 에이전트.</strong>
</p>
