# --- Daemon lifecycle ---
daemon-starting = 데몬을 시작하는 중...
daemon-stopped = LibreFang 데몬이 중지되었습니다.
kernel-booted = 커널이 부팅됨 ({ $provider }/{ $model })
models-available = { $count }개의 모델 사용 가능
agents-loaded = { $count }개의 에이전트가 로드됨
daemon-started-bg = 데몬이 백그라운드에서 시작됨
daemon-still-starting = 데몬이 백그라운드에서 실행되었으며 아직 시작하는 중
daemon-stopped-ok = 데몬이 중지됨
daemon-stopped-forced = 데몬이 중지됨 (강제)
daemon-error = 데몬 오류: { $error }
daemon-already-running = 데몬이 { $url }에서 이미 실행 중
daemon-already-running-fix = `librefang status`로 확인하거나 먼저 중지하십시오
daemon-not-running-start = 데몬이 실행 중이 아닙니다. librefang start로 시작하십시오
daemon-no-running-found = 실행 중인 데몬을 찾을 수 없음
daemon-no-running-found-fix = 실행 중입니까? librefang status로 확인하십시오
daemon-restarting = 데몬을 재시작하는 중...
daemon-no-running-starting = 실행 중인 데몬을 찾을 수 없습니다. 새 데몬을 시작합니다
daemon-bg-exited = 백그라운드 데몬이 정상 상태가 되기 전에 종료됨 ({ $status })
daemon-bg-exited-fix = 시작 로그를 확인하십시오: { $path }
daemon-bg-wait-fail = 백그라운드 데몬을 기다리는 중 실패
daemon-bg-wait-fail-fix = { $error }. 시작 로그를 확인하십시오: { $path }
daemon-launch-fail = 백그라운드 데몬 실행에 실패
daemon-no-running-auto = 실행 중인 데몬이 없습니다 — 지금 시작합니다...
daemon-started = 데몬이 시작됨
daemon-start-fail = 데몬을 시작할 수 없습니다: { $error }
daemon-start-fail-fix = 수동으로 시작하십시오: librefang start
shutdown-request-fail = 종료 요청 실패 ({ $status })
could-not-reach-daemon = 데몬에 연결할 수 없습니다: { $error }
# Issue #4693 — after `curl install.sh | sh` upgrades the binary without
# restarting the running daemon, `librefang restart` (new CLI) hits the old
# daemon's `/api/shutdown` and is rejected with 401 because the new CLI's
# Authorization header does not match the old daemon's expected key (typical
# trigger: locked vault, rotated `[api] api_key`, or freshly enabled
# dashboard credentials). Surface the cause + auto-fall-back to PID-based
# shutdown so users can move forward without hand-editing config.
shutdown-401-detected = 실행 중인 데몬이 종료 요청을 거부했습니다 (401 Unauthorized).
shutdown-401-explainer = 새 CLI가 현재 실행 중인 데몬에 인증할 수 없습니다. 이는 보통 `curl install.sh | sh`가 데몬을 다시 시작하지 않고 바이너리를 업그레이드한 후에 발생합니다 — 실행 중인 데몬이 다른 api_key로 시작되었거나, 이를 보관하는 볼트의 잠금을 해제할 수 없었기 때문입니다.
shutdown-401-fallback-attempt = PID 기반 중지로 대체합니다 (PID { $pid })...
shutdown-401-fallback-success = PID { $pid }을(를) 통해 데몬이 중지되었습니다
shutdown-401-fallback-fail = PID 기반 중지도 작동하지 않았습니다.
shutdown-401-fallback-fix = 데몬을 수동으로 중지한 다음 다시 시작하십시오:
    kill { $pid }    # 또는: 종료되지 않으면 kill -9 { $pid }
    librefang start
shutdown-401-no-pid-fix = { $path }에서 데몬 PID를 읽을 수 없습니다. `ps -ef | grep librefang`을 실행하여 찾은 다음 `kill <pid>`와 `librefang start`를 실행하십시오.

# --- Labels ---
label-api = API
label-dashboard = 대시보드
label-provider = 공급자
label-model = 모델
label-pid = PID
label-log = 로그
label-status = 상태
label-agents = 에이전트
label-data-dir = 데이터 디렉터리
label-uptime = 가동 시간
label-version = 버전
label-daemon = 데몬
label-id = ID
label-active-agents = 활성 에이전트
label-pairing-code = 페어링 코드
label-expires = 만료
label-yes = 예
label-no = 아니요
label-not-loaded = 로드되지 않음
label-current = 현재
label-channel = 채널
label-binary = 바이너리
label-latest = 최신
label-target = 대상
label-installed = 설치됨

# --- Hints ---
hint-open-dashboard = 브라우저에서 대시보드를 열거나 `librefang chat`을 실행하십시오
hint-stop-daemon = 데몬을 중지하려면 `librefang stop`을 사용하십시오
hint-tail-stop = Ctrl+C는 로그 추적을 중지합니다. 데몬은 계속 실행됩니다
hint-check-status = 준비 상태를 확인하려면 `librefang status`를 실행하십시오
hint-start-daemon = 다음으로 시작하십시오: librefang start
hint-start-daemon-cmd = 데몬 시작: librefang start
hint-or-chat = 또는 데몬 없이 작동하는 `librefang chat`을 사용해 보십시오
hint-non-interactive = 비대화형 터미널이 감지됨 — 빠른 모드로 실행 중
hint-non-interactive-wizard = 대화형 마법사를 사용하려면 (터미널에서) librefang init을 실행하십시오
hint-starting-chat = 채팅 세션을 시작하는 중...
hint-no-api-keys = LLM 공급자 API 키를 찾을 수 없음
hint-groq-free = Groq는 무료 등급을 제공합니다: https://console.groq.com
hint-ollama-local = 또는 로컬 모델을 위해 Ollama를 설치하십시오: https://ollama.com
hint-gemini-free = Gemini는 무료 등급을 제공합니다: https://aistudio.google.com
hint-deepseek-free = DeepSeek는 500만 무료 토큰을 제공합니다: https://platform.deepseek.com
guide-title = 빠른 설정
guide-free-providers-title = 시작할 무료 제공자를 선택하십시오 (2분 설정):
guide-get-free-key = 무료 API 키 받기
guide-paste-key-placeholder = 여기에 API 키를 붙여넣으십시오
guide-setting-up = 설정 중
guide-testing-key = 키 테스트 중...
guide-key-verified = ✓ 키 검증됨!
guide-test-key-unverified = ⚠ 검증할 수 없음 (작동할 수도 있음)
guide-help-select = ↑↓ 탐색  Enter 선택  s/Esc 건너뛰기
guide-help-paste = 키 붙여넣기 + Enter  Esc 뒤로
guide-help-wait = 잠시 기다려 주십시오...
guide-paste-key-hint = 브라우저에서 API 키를 복사하여 아래에 붙여넣으십시오.
hint-could-not-open-browser-visit = 브라우저를 열 수 없습니다. 방문하십시오: { $url }
hint-chat-with-agent = 채팅: librefang chat { $name }
hint-agent-lost-on-exit = 참고: 이 프로세스가 종료되면 에이전트가 사라집니다
hint-persistent-agents = 지속 에이전트를 사용하려면 먼저 `librefang start`를 실행하십시오
hint-url-copied = URL이 클립보드에 복사되었습니다
hint-doctor-repair = 자동 수정을 시도하려면 `librefang doctor --repair`를 실행하십시오
hint-run-start = 데몬을 실행하려면 `librefang start`를 실행하십시오
hint-config-edit = 수정 방법: librefang config edit
hint-set-key = 또는 실행하십시오: librefang config set-key groq

# --- Init ---
init-quick-success = LibreFang이 초기화되었습니다 (빠른 모드)
init-interactive-success = LibreFang 초기화 완료!
init-cancelled = 설정이 취소되었습니다.
init-next-start = 데몬 시작:        librefang start
init-next-chat = 채팅:             librefang chat

# --- Error messages ---
error-home-dir = 홈 디렉터리를 확인할 수 없습니다
error-create-dir = { $path } 생성에 실패했습니다
error-create-dir-fix = { $path }의 권한을 확인하십시오
error-write-config = 구성 파일 쓰기에 실패했습니다
error-config-created = 생성됨: { $path }
error-config-exists = 구성이 이미 존재합니다: { $path }

# --- Daemon communication errors ---
error-daemon-returned = 데몬이 오류를 반환했습니다 ({ $status })
error-daemon-returned-fix = 다음으로 데몬 로그를 확인하십시오: librefang logs --follow
error-request-timeout = 요청 시간이 초과되었습니다
error-request-timeout-fix = 에이전트가 복잡한 요청을 처리하고 있을 수 있습니다. 다시 시도하거나 `librefang status`를 확인하십시오
error-connect-refused = 데몬에 연결할 수 없습니다
error-connect-refused-fix = 데몬이 실행 중입니까? 다음으로 시작하십시오: librefang start
error-daemon-comm = 데몬 통신 오류: { $error }
error-daemon-comm-fix = `librefang status`를 확인하거나 다시 시작하십시오: librefang start

# --- Boot errors ---
error-boot-config = 구성 파싱에 실패했습니다
error-boot-config-fix = config.toml 구문을 확인하십시오: librefang config show
error-boot-db = 데이터베이스 오류 (파일이 잠겨 있을 수 있습니다)
error-boot-db-fix = 다른 LibreFang 프로세스가 실행 중인지 확인하십시오: librefang status
error-boot-auth = LLM 공급자 인증에 실패했습니다
error-boot-auth-fix = `librefang doctor`를 실행하여 API 키 구성을 확인하십시오
error-boot-generic = 커널 부팅 실패: { $error }
error-boot-generic-fix = `librefang doctor`를 실행하여 문제를 진단하십시오

# --- Require daemon ---
error-require-daemon = `librefang { $command }`은(는) 실행 중인 데몬이 필요합니다
error-require-daemon-fix = 데몬을 시작하십시오: librefang start

# --- Provider detection ---
detected-provider = { $display } 감지됨 ({ $env_var })
detected-ollama = 로컬에서 실행 중인 Ollama 감지됨 (API 키 불필요)

# --- Desktop app ---

# --- Dashboard ---
dashboard-opening = { $url }에서 대시보드 여는 중

# --- Agent commands ---
agent-spawned = 에이전트 '{ $name }' 생성됨
agent-spawned-inprocess = 에이전트 '{ $name }' 생성됨 (인프로세스)
agent-spawn-failed = 생성 실패: { $error }
agent-spawn-agent-failed = 에이전트 생성 실패: { $error }
agent-killed = 에이전트 { $id } 종료됨.
agent-kill-failed = 에이전트 종료 실패: { $error }
agent-invalid-id = 잘못된 에이전트 ID: { $id }
agent-no-agents = 실행 중인 에이전트가 없습니다.
agent-spawn-success = 에이전트가 성공적으로 생성되었습니다!
agent-spawn-inprocess-mode = 에이전트 생성됨 (인프로세스 모드).
agent-note-lost = 참고: 이 프로세스가 종료되면 에이전트가 사라집니다.
agent-note-persistent = 지속 에이전트를 사용하려면 먼저 `librefang start`를 실행하십시오.
section-agent-templates = 사용 가능한 에이전트 템플릿

# --- Manifest errors ---
manifest-not-found = 매니페스트 파일을 찾을 수 없음: { $path }
manifest-not-found-fix = 대신 `librefang agent new`를 사용하여 템플릿에서 생성하십시오
error-reading-manifest = 매니페스트 읽기 오류: { $error }

# --- Status ---
section-daemon-status = LibreFang 데몬 상태
section-status-inprocess = LibreFang 상태 (인프로세스)
section-active-agents = 활성 에이전트
section-persisted-agents = 저장된 에이전트
label-daemon-not-running = 실행 중 아님
label-home = 홈
label-platform = 플랫폼
label-sessions = 세션
label-memory = 메모리
label-running = 실행 중
label-response = 응답
label-checks = 검사
section-status-locked = 제한됨 (API 키 필요)
hint-status-locked = 에이전트 / 세션 / 메모리를 보려면 ~/.librefang/config.toml에 `api_key`를 설정하십시오.
warn-public-bind = 공개 바인딩됨
warn-key-missing = 설정되지 않음
section-recent-errors = 최근 오류 (daemon.log)
section-verbose = 세부 정보
label-auth = 인증
label-mcp = MCP 서버
label-peers = OFP 노드
label-channels = 채널
label-skills = 스킬
label-hands = 핸드
label-config-warnings = 구성 경고
auth-none = 없음 (익명)
auth-api-key = API 키
auth-dashboard-login = 대시보드 로그인

# --- Doctor ---
doctor-title = LibreFang 진단
doctor-all-passed = 모든 검사를 통과했습니다! LibreFang을 사용할 준비가 되었습니다.
doctor-repairs-applied = 복구를 적용했습니다. `librefang doctor`를 다시 실행하여 검증하십시오.
doctor-some-failed = 일부 검사에 실패했습니다.
doctor-no-api-keys = LLM 제공자 API key를 찾을 수 없습니다!
section-getting-api-key = API key 받기 (무료 등급)

# --- Security ---
section-security-status = 보안 상태
label-audit-trail = 감사 추적
label-taint-tracking = 오염 추적
label-wasm-sandbox = WASM 샌드박스
label-wire-protocol = 와이어 프로토콜
label-api-keys = API key
label-manifests = 매니페스트
value-audit-trail = Merkle 해시 체인 (SHA-256)
value-taint-tracking = 정보 흐름 레이블
value-wasm-sandbox = 이중 미터링 (fuel + epoch)
value-wire-protocol = OFP HMAC-SHA256 상호 인증
value-api-keys = Zeroizing<String> (drop 시 자동 삭제)
value-manifests = Ed25519 서명됨
audit-verified = 감사 추적 무결성이 검증되었습니다 (Merkle 체인 유효).
audit-failed = 감사 추적 무결성 검사에 실패했습니다.

# --- Health ---
health-ok = 데몬이 정상입니다
health-not-running = 데몬이 실행 중이 아닙니다.

# --- Channel setup ---
channel-none-configured = 구성된 채널이 없습니다.
channel-use-setup-hint = `librefang channel setup`을 사용하여 채널을 추가하십시오.
channel-reloaded = 채널을 다시 불러왔습니다 (사이드카 { $started }개 시작됨).
channel-registry-empty = 데몬의 채널 레지스트리가 비어 있습니다.
channel-install-sdk-hint = 어댑터가 카탈로그에 표시되도록 사이드카 SDK를 설치하십시오:
channel-install-sdk-cmd =   pip install librefang-sdk
channel-rerun-setup-hint = 그런 다음 `librefang channel setup`을 다시 실행하십시오.
channel-all-configured = 사용 가능한 모든 채널이 이미 구성되어 있습니다.
channel-see-list-hint = `librefang channel list`로 채널을 확인하거나,
channel-remove-entry-hint = `librefang channel rm <name>`으로 먼저 항목을 제거하십시오.
channel-pick-setup = 설정할 채널을 선택하십시오:
channel-choice-prompt = 선택 [1]: 
channel-unknown-error = 알 수 없는 채널: { $name }
channel-unknown-error-fix = `librefang channel list`를 실행하여 사용 가능한 어댑터를 확인하십시오.
channel-no-configurable-fields = `{ $name }`에는 구성 가능한 필드가 없습니다 — 입력받을 항목이 없습니다.
channel-hot-reload-manual-hint = (config.toml을 직접 편집한 경우 `librefang channel reload`로 핫 리로드할 수 있습니다.)
channel-prompt-secret-keep =   { $label } ({ $key }) [설정됨 — 유지하려면 비워 두십시오]: 
channel-prompt-default =   { $label } ({ $key }) [{ $current }]: 
channel-prompt-required =   { $label } ({ $key }) *: 
channel-prompt-optional =   { $label } ({ $key }): 
channel-save-rejected = `{ $name }` 저장이 거부되었습니다: { $error }
channel-save-rejected-fix = 수정된 값으로 다시 실행하거나, 자세한 내용은 데몬 로그를 확인하십시오.
channel-saved-restart-required = ✓ `{ $name }`을(를) 저장했습니다 — 변경 사항을 적용하려면 데몬을 다시 시작하십시오.
channel-saved-hot-reload = ✓ `{ $name }`을(를) 저장했습니다 — 핫 리로드가 적용되었습니다.
channel-env-shadowing-warn = 경고: 셸 환경 변수가 이 토큰들을 가리고 있습니다 — 새 값을 적용하려면 해당 변수를 해제하고 다시 시작하십시오: { $keys }
channel-config-read-fail = { $path }을(를) 읽을 수 없습니다: { $error }
channel-config-read-fail-fix = `librefang init`을 실행하여 설정 파일을 생성하십시오.
channel-config-parse-fail = { $path }을(를) 파싱할 수 없습니다: { $error }
channel-config-parse-fail-fix = TOML 구문을 수정하고 다시 시도하십시오.
channel-no-entries-to-remove = config.toml에 [[sidecar_channels]] 항목이 없습니다 — 제거할 것이 없습니다.
channel-no-entry-with-name = name="{ $name }"인 [[sidecar_channels]] 항목이 없습니다.
channel-config-write-fail = { $path } 쓰기에 실패했습니다: { $error }
channel-config-write-fail-fix = 파일 시스템 권한을 확인하십시오.
channel-removed-entries = ✓ `{ $name }` 이름의 [[sidecar_channels]] 항목 { $count }개를 제거했습니다.
channel-hot-reloaded-daemon =   데몬을 핫 리로드했습니다.
channel-reload-status-warn =   다시 불러오기가 { $status }을(를) 반환했습니다: 변경 사항은 다음 데몬 재시작 시 적용됩니다.
channel-reload-contact-fail-warn =   다시 불러오기를 위해 데몬에 연결할 수 없습니다 ({ $error }); 변경 사항은 다음 시작 시 적용됩니다.
channel-reload-daemon-offline =   데몬이 실행 중이 아닙니다; 변경 사항은 다음 시작 시 적용됩니다.
# --- Vault ---
vault-initialized = 자격 증명 볼트를 초기화했습니다.
vault-not-initialized = 볼트가 초기화되지 않았습니다.
vault-not-init-run = 볼트가 초기화되지 않았습니다. 실행: librefang vault init
vault-unlock-failed = 볼트를 잠금 해제할 수 없습니다: { $error }
vault-empty-value = 값이 비어 있습니다 — 저장되지 않았습니다.
vault-stored = '{ $key }'을(를) 볼트에 저장했습니다.
vault-store-failed = 저장에 실패했습니다: { $error }
vault-removed = '{ $key }'을(를) 볼트에서 제거했습니다.
vault-key-not-found = 볼트에서 키 '{ $key }'을(를) 찾을 수 없습니다.
vault-remove-failed = 제거하지 못했습니다: { $error }
vault-rotate-no-vault = 볼트 파일을 찾을 수 없습니다. 먼저 `librefang vault init`을(를) 실행하십시오.
vault-rotate-old-key-missing = LIBREFANG_VAULT_KEY_OLD가 설정되지 않았습니다. 교체하기 전에 현재 마스터 키(32바이트의 base64)를 제공하십시오.
vault-rotate-new-key-missing = LIBREFANG_VAULT_KEY_NEW가 설정되지 않았습니다. 새 마스터 키(32바이트의 base64)를 제공하거나, stdin에서 읽으려면 --from-stdin을 전달하십시오.
vault-rotate-stdin-read-failed = stdin에서 새 키를 읽지 못했습니다: { $error }
vault-rotate-stdin-empty = stdin에서 읽은 새 키가 비어 있습니다.
vault-rotate-same-key = LIBREFANG_VAULT_KEY_OLD와 새 키가 동일합니다 — 동일한 키로 교체를 거부합니다.
vault-rotate-old-key-invalid = LIBREFANG_VAULT_KEY_OLD가 유효한 32바이트 base64 키가 아닙니다: { $error }
vault-rotate-new-key-invalid = 새 키가 유효한 32바이트 base64 키가 아닙니다: { $error }
vault-rotate-unlock-failed = OLD 키로 볼트를 잠금 해제하지 못했습니다: { $error }. LIBREFANG_VAULT_KEY_OLD가 볼트를 처음 암호화한 키와 일치하는지 확인하십시오.
vault-rotate-sentinel-failed = OLD 키로 볼트 센티넬 검증에 실패했습니다: { $error }
vault-rotate-rewrap-failed = 새 키로 볼트를 재암호화하지 못했습니다: { $error }. 원본 볼트 파일은 변경되지 않았습니다.
vault-rotate-success = 새 마스터 키로 볼트를 재암호화했습니다(사용자 항목 { $count }개 보존됨).
vault-rotate-next-step = 다음: 데몬을 다시 시작하기 전에 LIBREFANG_VAULT_KEY를 새 값으로 설정하십시오.

# --- Cron ---
cron-created = 크론 작업이 생성되었습니다: { $id }
cron-create-failed = 크론 작업을 생성하지 못했습니다: { $error }
cron-deleted = 크론 작업 { $id }이(가) 삭제되었습니다.
cron-delete-failed = 크론 작업을 삭제하지 못했습니다: { $error }
cron-toggled = 크론 작업 { $id }에 { $action } 작업을 수행했습니다.
cron-toggle-failed = 크론 작업에 { $action } 작업을 수행하지 못했습니다: { $error }

# --- Automation ---
automation-workflow-none = 등록된 워크플로가 없습니다.
automation-workflow-file-not-found = 워크플로 파일을 찾을 수 없습니다: { $path }
automation-workflow-read-error = 워크플로 파일을 읽는 중 오류 발생: { $error }
automation-workflow-invalid-json = 잘못된 JSON: { $error }
automation-workflow-created = 워크플로가 성공적으로 생성되었습니다!
automation-workflow-created-id =   ID: { $id }
automation-workflow-create-failed = 워크플로 생성 실패: { $error }
automation-workflow-completed = 워크플로가 완료되었습니다!
automation-workflow-run-id =   실행 ID: { $id }
automation-workflow-failed = 워크플로 실패: { $error }
automation-trigger-none = 등록된 트리거가 없습니다.
automation-trigger-invalid-pattern = 잘못된 패턴 JSON: { $error }
automation-trigger-created = 트리거가 성공적으로 생성되었습니다!
automation-trigger-created-id =   트리거 ID: { $id }
automation-trigger-created-agent =   에이전트 ID:   { $agent_id }
automation-trigger-created-target =   대상:     { $target }
automation-trigger-create-failed = 트리거 생성 실패: { $error }
automation-trigger-deleted = 트리거 { $id }이(가) 삭제되었습니다.
automation-trigger-delete-failed = 트리거 삭제 실패: { $error }
automation-trigger-get-failed = 트리거 조회 실패: { $error }
automation-trigger-update-failed = 트리거 업데이트 실패: { $error }
automation-trigger-updated = 트리거 { $id }이(가) 업데이트되었습니다.
automation-trigger-toggle-failed = 트리거 { $action } 실패: { $error }
automation-trigger-toggled = 트리거 { $id }이(가) { $action }되었습니다.
automation-trigger-info-id = 트리거 ID:    { $id }
automation-trigger-info-agent = 에이전트 ID:      { $id }
automation-trigger-info-pattern = 패턴:       { $pattern }
automation-trigger-info-prompt = 프롬프트:        { $prompt }
automation-trigger-info-enabled = 활성화 여부:     { $enabled }
automation-trigger-info-fires = 발동 횟수:       { $count }
automation-trigger-info-max-fires = 최대 발동 횟수:  { $count }
automation-trigger-info-target = 대상 에이전트:   { $agent }
automation-trigger-info-cooldown = 쿨다운:          { $secs }s
automation-trigger-info-session = 세션 모드:       { $mode }
automation-unlimited = 무제한
automation-cron-none = 예약된 작업이 없습니다.

label-header-steps = 단계
label-header-trigger-id = 트리거 ID
label-header-agent-id = 에이전트 ID
label-header-fires = 발동 횟수
label-header-pattern = 패턴
label-header-schedule = 일정
label-header-prompt = 프롬프트

# --- Approvals ---
approval-responded = 승인 { $id }을(를) { $action } 처리했습니다.
approval-failed = 승인 { $action } 처리에 실패했습니다: { $error }

# --- Memory ---
memory-set = 에이전트 '{ $agent }'에 { $key }을(를) 설정했습니다.
memory-set-failed = 메모리 설정에 실패했습니다: { $error }
memory-deleted = 에이전트 '{ $agent }'의 키 '{ $key }'을(를) 삭제했습니다.
memory-delete-failed = 메모리 삭제에 실패했습니다: { $error }

# --- Devices ---
section-device-pairing = 기기 페어링
device-scan-qr = LibreFang 모바일 앱으로 이 QR 코드를 스캔하십시오:
device-removed = 기기 { $id }이(가) 제거되었습니다.
device-remove-failed = 기기 제거에 실패했습니다: { $error }

# --- Webhooks ---
webhook-created = 웹훅이 생성되었습니다: { $id }
webhook-create-failed = 웹훅 생성에 실패했습니다: { $error }
webhook-deleted = 웹훅 { $id }이(가) 삭제되었습니다.
webhook-delete-failed = 웹훅 삭제에 실패했습니다: { $error }
webhook-test-ok = 웹훅 { $id } 테스트 페이로드를 성공적으로 전송했습니다.
webhook-test-failed = 웹훅 테스트에 실패했습니다: { $error }

# --- Models ---
model-set-success = 기본 모델이 다음으로 설정되었습니다: { $model }
model-set-failed = 모델 설정에 실패했습니다: { $error }
model-no-catalog = 카탈로그에 모델이 없습니다.
section-select-model = 모델 선택
model-out-of-range = 범위를 벗어난 숫자입니다 (1-{ $max })
model-none-found = 모델을 찾을 수 없습니다.
model-prompt-selection =   숫자 또는 모델 ID를 입력하십시오: 


# --- Config ---
config-no-file = 구성 파일을 찾을 수 없습니다
config-no-file-fix = 먼저 `librefang init`을(를) 실행하십시오
config-read-failed = 구성을 읽지 못했습니다: { $error }
config-parse-error = 구성 구문 분석 오류: { $error }
config-parse-fix = config.toml 구문을 수정하거나 `librefang config edit`을(를) 실행하십시오
config-parse-fix-alt = 먼저 config.toml 구문을 수정하십시오
config-key-not-found = 키를 찾을 수 없습니다: { $key }
config-key-path-not-found = 키 경로를 찾을 수 없습니다: { $key }
config-empty-key = 빈 키
config-section-not-scalar = '{ $key }'은(는) 스칼라가 아니라 섹션입니다
config-section-not-scalar-fix = 점 표기법을 사용하십시오: { $key }.field_name
config-parent-not-table = '{ $key }'의 상위 항목이 테이블이 아닙니다
config-serialize-failed = 구성 직렬화에 실패했습니다: { $error }
config-write-failed = 구성 쓰기에 실패했습니다: { $error }
config-set-kv = { $key } = { $value } 설정됨
config-removed-key = 키 제거됨: { $key }
config-no-key = 키가 제공되지 않았습니다. 취소되었습니다.
config-saved-key = { $env_var }을(를) ~/.librefang/.env에 저장했습니다
config-save-key-failed = 키 저장에 실패했습니다: { $error }
config-removed-env = { $env_var }을(를) ~/.librefang/.env에서 제거했습니다
config-remove-key-failed = 키 제거에 실패했습니다: { $error }
config-env-not-set = { $env_var }이(가) 설정되지 않았습니다
config-set-key-hint = 설정하십시오: librefang config set-key { $provider }
config-update-key-hint = 키 업데이트: librefang config set-key { $provider }
config-no-file-found = 다음 위치에서 구성을 찾을 수 없습니다: { $path }
config-run-init-hint = `librefang init`을 실행하여 생성하십시오.
config-read-error = 구성 읽기 오류: { $error }
config-editor-exit = 편집기가 다음으로 종료되었습니다: { $status }
config-editor-open-fail = 편집기 '{ $editor }' 열기에 실패했습니다: { $error }
config-editor-env-hint = 선호하는 편집기로 $EDITOR를 설정하십시오.
config-val-exceeds-i64 = 값 { $value }이(가) i64::MAX({ $max })를 초과합니다; TOML은 이 한계를 넘는 부호 없는 정수를 저장할 수 없습니다
config-invalid-integer = '{ $raw }'은(는) 유효한 정수가 아닙니다
config-paste-api-key-prompt =   { $provider } API 키를 붙여넣으십시오: 
config-testing-key =   키 테스트 중... 
config-testing-provider-key =   { $provider } ({ $env_var }) 테스트 중... 
config-test-ok = 정상
config-test-failed = 실패 (401/403)
config-test-unverified = 검증할 수 없음 (정상 동작할 수도 있음)


# --- Hand commands ---
hand-install-deps-success = 핸드 '{ $id }'의 의존성이 설치되었습니다.
hand-paused = 핸드 인스턴스 '{ $label } (instance: { $instance_id })'가 일시 중지되었습니다.
hand-resumed = 핸드 인스턴스 '{ $label } (instance: { $instance_id })'가 재개되었습니다.

# --- Daemon notify ---

# --- System info ---
section-system-info = LibreFang 시스템 정보

# --- Uninstall ---
uninstall-warning = 이 작업은 시스템에서 LibreFang을 완전히 제거합니다.
uninstall-remove-data-kept =   • { $path }의 데이터 제거 (config 파일은 유지)
uninstall-remove-all =   • { $path } 제거
uninstall-remove-binary =   • 바이너리 제거: { $path }
uninstall-remove-cargo-binary =   • cargo 바이너리 제거: { $path }
uninstall-remove-autostart =   • 자동 시작 항목 제거 (있는 경우)
uninstall-clean-path =   • 셸 config에서 PATH 정리 (있는 경우)
uninstall-confirm-prompt =   확인하려면 'uninstall'을 입력하십시오: 
uninstall-goodbye = LibreFang이 제거되었습니다. 안녕히 가십시오!
uninstall-cancelled = 취소되었습니다.
uninstall-stopping-daemon = 실행 중인 데몬을 중지하는 중...
uninstall-removed = { $path }를 제거했습니다
uninstall-remove-failed = { $path } 제거에 실패했습니다: { $error }
uninstall-removed-data-kept = 데이터를 제거했습니다 (config 파일은 유지)
uninstall-removed-autostart-win = Windows 자동 시작 레지스트리 항목을 제거했습니다
uninstall-removed-launch-agent = macOS launch agent를 제거했습니다
uninstall-remove-launch-fail = launch agent 제거에 실패했습니다: { $error }
uninstall-removed-autostart-linux = Linux 자동 시작 항목을 제거했습니다
uninstall-remove-autostart-fail = 자동 시작 항목 제거에 실패했습니다: { $error }
uninstall-removed-systemd = systemd 사용자 서비스를 제거했습니다
uninstall-remove-systemd-fail = systemd 서비스 제거에 실패했습니다: { $error }
uninstall-cleaned-path = { $path }에서 PATH를 정리했습니다
uninstall-cleaned-path-win = Windows 사용자 환경에서 PATH를 정리했습니다

# --- Reset ---
reset-success = { $path }을(를) 제거했습니다
reset-fail = { $path } 제거에 실패했습니다: { $error }

# --- Logs ---
log-following = --- { $path } 추적 중 (Ctrl+C로 중지) ---

# --- Extracted from Rust sources ---
init-error-create-data-dir = 데이터 디렉터리 생성 오류: { $error }
init-upgrade-existing = 기존 설치가 감지되었습니다 — 설정을 보존하기 위해 업그레이드를 실행합니다.
init-upgrade-fresh-hint = 새로 시작하려면 ~/.librefang/config.toml을 제거한 후 `librefang init`을 다시 실행하십시오.
init-upgrade-no-config = 업그레이드할 항목이 없습니다 — config.toml을 찾을 수 없습니다. 먼저 `librefang init`을 실행하십시오.
init-upgrade-registry-synced = 레지스트리가 동기화되었습니다
init-upgrade-registry-failed = 레지스트리 동기화에 실패했습니다 (네트워크 문제?) — 캐시된 콘텐츠로 계속합니다
init-upgrade-config-up-to-date = 구성이 이미 최신 상태입니다 — 새 필드가 추가되지 않았습니다
init-upgrade-sections-added = { $count }개의 새 구성 섹션을 추가했습니다:
init-upgrade-legacy-openclaw = 레거시 ~/.openclaw 설치가 감지되었습니다.
init-upgrade-legacy-openclaw-hint = 데이터를 마이그레이션하려면 `librefang migrate --from openclaw`를 실행하십시오.
init-upgrade-approval-warning = require_approval 목록에 "shell_exec"만 포함되어 있습니다. 파일 작업(file_write, file_delete)은 이제 기본적으로 승인이 필요합니다.
init-upgrade-approval-hint = 활성화하려면: config.toml의 require_approval에 "file_write"와 "file_delete"를 추가하십시오
init-upgrade-success-summary = 업그레이드 완료!
init-upgrade-title = LibreFang 설치 업그레이드 중
init-upgrade-progress-label = 업그레이드 중
init-upgrade-backing-up = 구성 백업 중
init-upgrade-backup-success = 구성을 backups/{ $name }에 백업했습니다
init-upgrade-syncing-registry = 레지스트리 동기화 중
init-upgrade-initializing-vault-git = 볼트/git 초기화 중
init-upgrade-merging-config = 구성 필드 병합 중
init-upgrade-failed-read = 업그레이드 중단됨: config.toml 읽기 실패: { $error }
init-upgrade-failed-parse = 업그레이드 중단됨: config.toml 파싱 실패: { $error }
init-upgrade-backup-saved-hint = 원본 구성이 backups/{ $name }에 저장되었습니다
init-upgrade-failed-parse-template = 업그레이드 중단됨: 기본 구성 템플릿 파싱 실패: { $error }
init-upgrade-failed-write = 업그레이드 중단됨: 구성 쓰기 실패: { $error }
init-upgrade-steps-complete = 업그레이드 단계 완료
label-backup = 백업
label-new-fields = 새 필드

auth-chatgpt-device-requested = 기기 인증을 요청했습니다.
auth-chatgpt-device-open-url = 아무 브라우저에서나 이 URL을 여십시오:\n  { $url }\n
auth-chatgpt-device-one-time-code = 이 일회용 코드를 입력하십시오:\n  { $code }\n
auth-chatgpt-device-do-not-share = 이 코드를 공유하지 마십시오.
auth-chatgpt-device-waiting = 승인을 기다리는 중...
auth-chatgpt-switching-browser = \n표준 브라우저 로그인 흐름으로 전환하는 중...\n
auth-chatgpt-opening-browser = OpenAI 인증을 위해 브라우저를 여는 중...
auth-chatgpt-open-manually-hint = 브라우저가 열리지 않으면 다음 주소를 방문하십시오:\n  { $url }\n
auth-chatgpt-open-browser-failed = 브라우저를 자동으로 열 수 없습니다: { $error }
auth-chatgpt-open-manually = 수동으로 여십시오: { $url }
auth-chatgpt-tokens-saved = \nChatGPT 토큰을 { $path }에 저장했습니다
auth-chatgpt-detecting-model = 사용 가능한 최적의 모델을 감지하는 중...
auth-chatgpt-selected-model = 선택된 모델: { $model }
auth-chatgpt-config-updated = config.toml 업데이트됨: provider = "chatgpt", model = "{ $model }"
auth-chatgpt-starting-flow = ChatGPT 인증 플로우를 시작하는 중...\n
auth-chatgpt-complete = ChatGPT 인증이 완료되었습니다.
auth-chatgpt-failed = ChatGPT 인증에 실패했습니다: { $error }

auth-pool-config-not-array = config.toml `credential_pools`가 존재하지만 테이블 배열이 아닙니다
auth-pool-daemon-error-fallback = 데몬이 HTTP { $status }을(를) 반환했습니다 — config.toml 보기로 대체합니다
auth-pool-daemon-connect-fallback = { $url }에서 데몬 조회에 실패했습니다: { $error } — config.toml 보기로 대체합니다
auth-pool-no-config-offline = { $path }에 설정이 없고 데몬이 실행 중이 아닙니다.
auth-pool-config-load-failed = 설정을 불러오지 못했습니다: { $error }
auth-pool-none-configured = 구성된 자격 증명 풀이 없습니다.
auth-pool-invalid-env-name = `{ $env_var }`은(는) 유효한 환경 변수 이름이 아닙니다. 대문자, 숫자, 밑줄을 사용해야 합니다 (예: OPENAI_API_KEY_2).
auth-pool-env-empty = 환경 변수 `{ $env_var }`이(가) 설정되어 있지만 비어 있습니다.
auth-pool-env-empty-fix = 풀 항목을 추가하기 전에 API 키로 설정하십시오, 예:\n  export { $env_var }=sk-…\n그런 다음 재시도하십시오.
auth-pool-env-not-set = 환경 변수 `{ $env_var }`이(가) 현재 셸에 설정되어 있지 않습니다.
auth-pool-env-not-set-fix = 풀 항목을 추가하기 전에 내보내십시오, 예:\n  export { $env_var }=sk-…\n그런 다음 재시도하십시오. (데몬은 부팅 시 자체 환경에서 이를 읽으므로 거기에도 내보냈는지 확인하십시오.)
auth-pool-keys-not-array = `{ $provider }`의 풀에 테이블 배열이 아닌 `keys` 필드가 있습니다.
auth-pool-key-duplicate = env_var `{ $env_var }`을(를) 가진 키가 이미 `{ $provider }` 제공자의 풀에 존재합니다.
auth-pool-key-added = 키 `{ $label }`(env={ $env_var }, priority={ $priority })을(를) `{ $provider }`의 풀에 추가했습니다. 적용하려면 데몬을 재시작하거나 설정을 핫 리로드하십시오.
auth-pool-not-configured = `{ $provider }` 제공자에 구성된 자격 증명 풀이 없습니다.
auth-pool-no-keys-field = `{ $provider }`의 풀에 keys 배열이 없습니다.
auth-pool-key-not-found = `{ $provider }`의 풀에서 env_var `{ $env_var }`을(를) 가진 키를 찾을 수 없습니다.
auth-pool-key-removed-pool-empty = `{ $provider }`의 풀에서 키 `{ $env_var }`을(를) 제거했습니다. 풀이 이제 비어 있어 완전히 제거되었습니다. 적용하려면 데몬을 재시작하거나 config를 핫 리로드하십시오.
auth-pool-key-removed = `{ $provider }`의 풀에서 키 `{ $env_var }`을(를) 제거했습니다. 적용하려면 데몬을 재시작하거나 config를 핫 리로드하십시오.
auth-pool-unknown-strategy = 알 수 없는 전략 `{ $strategy }`입니다. 유효한 값: fill_first, round_robin, random, least_used.
auth-pool-strategy-set = `{ $provider }`의 풀 전략을 `{ $strategy }`(으)로 설정했습니다. 적용하려면 데몬을 재시작하거나 config를 핫 리로드하십시오.
vault-empty = 볼트가 비어 있습니다.
vault-stored-count = 저장된 자격 증명 ({ $count }):

# --- Scanned & Extracted keys ---
# init.rs
init-upgrade-failed-create-backups-dir = backups 디렉터리 생성에 실패했습니다: { $error }
init-upgrade-failed-backup-config = config 백업에 실패했습니다: { $error }
init-error-write-config-example = config.example.toml을(를) 쓸 수 없습니다: { $error }

# acp.rs
acp-attached-uds = librefang acp: 데몬에 연결됨 (UDS { $path })
acp-attached-pipe = librefang acp: 데몬에 연결됨 (named pipe)
acp-in-process = librefang acp: 인프로세스 커널 (데몬이 감지되지 않음)
acp-error-boot-kernel = 커널 부팅에 실패했습니다: { $error }
acp-error-resolve-agent = 에이전트 '{ $name }' 확인에 실패했습니다: { $error }
acp-error-server = ACP 서버 오류: { $error }
acp-error-uds-connect = { $path }에서 ACP UDS 연결에 실패했습니다: { $error }
acp-error-pipe-connect = { $name }에서 ACP named-pipe 연결에 실패했습니다: { $error }


# auth.rs
auth-write-failed = { $path } 쓰기에 실패했습니다: { $error }
auth-password-empty = 비밀번호는 비워 둘 수 없습니다.
auth-passwords-mismatch = 비밀번호가 일치하지 않습니다.
auth-password-hash-failed = 비밀번호 해시에 실패했습니다: { $error }
vault-enter-value-prompt = { $key }의 값을 입력하십시오: 
auth-enter-password-prompt = 비밀번호를 입력하십시오: 
auth-confirm-password-prompt = 비밀번호 확인: 

# agent.rs
agent-spawn-choose-target-or-template = 위치 인수 대상 또는 `--template` 중 하나만 선택하고 둘 다 사용하지 마십시오.
agent-spawn-choose-target-or-template-fix = `librefang spawn coder` 또는 `librefang spawn --template agents/custom/my-agent.toml`를 사용하십시오.
agent-spawn-name-requires-template = `--name`에는 템플릿 이름 또는 매니페스트 경로가 필요합니다.
agent-spawn-name-requires-template-fix = `librefang spawn coder --name backend-coder` 또는 `librefang spawn --template path/to/agent.toml --name backend-coder`를 사용하십시오.
agent-spawn-dry-run-requires-template = 테스트 실행에는 템플릿 이름 또는 매니페스트 경로가 필요합니다.
agent-spawn-dry-run-requires-template-fix = `librefang spawn coder --dry-run` 또는 `librefang spawn --template path/to/agent.toml --dry-run`을 사용하십시오.
agent-spawn-template-or-path-not-found = 템플릿 또는 매니페스트 경로를 찾을 수 없습니다: { $target }
agent-spawn-template-or-path-not-found-fix = `librefang agent new`를 실행하여 템플릿을 둘러보거나 유효한 매니페스트 경로를 전달하십시오.
agent-manifest-parse-failed = { $source }에서 에이전트 매니페스트를 파싱하지 못했습니다: { $error }
agent-manifest-parse-failed-fix = 매니페스트 TOML 구문과 필수 필드를 확인하십시오.
agent-manifest-serialize-failed = 업데이트된 매니페스트를 직렬화하지 못했습니다: { $error }
agent-dry-run-title = 에이전트 테스트 실행
agent-dry-run-success = 매니페스트를 성공적으로 파싱했습니다. 생성된 에이전트는 없습니다.
agent-delete-warning-text = 경고: 에이전트 "{ $name }"을(를) 삭제하면 표준 UUID와
    연결된 모든 메모리 및 세션이 영구적으로 제거됩니다.
    이 작업은 되돌릴 수 없습니다.
label-confirm-prompt = 확인하시겠습니까?
label-aborted = 중단되었습니다.
agent-delete-no-uuid = 에이전트 이름 '{ $name }'에 기록된 표준 UUID가 없어 삭제할 항목이 없습니다.
agent-deleted-success = 에이전트 "{ $name }"이(가) 삭제되었습니다 (표준 UUID 제거됨).
agent-delete-failed-with-reason = 에이전트를 삭제하지 못했습니다: { $error }
agent-reset-uuid-warning-text = 경고: "{ $name }"의 표준 UUID를 재설정하면 현재 UUID에 연결된
    모든 세션과 메모리가 고아 상태가 됩니다. 이 이름으로 다음에
    생성되는 에이전트는 새 UUID로 시작합니다. 이 작업은 되돌릴 수 없습니다.
agent-reset-uuid-success = "{ $name }"의 표준 UUID가 초기화 재설정되었습니다 (이전 값: { $previous }).
agent-reset-uuid-failed-with-reason = 표준 UUID를 초기화 재설정하지 못했습니다: { $error }
agent-reset-uuid-not-found = 에이전트 이름 '{ $name }'에 기록된 표준 UUID가 없습니다.
agent-merge-history-not-implemented = merge-history는 아직 구현되지 않았습니다 (refs #4614 후속 작업).
    { $from }에서 에이전트 "{ $name }"의 표준 UUID로 세션/메모리를
    재할당하려면 메모리 substrate에서 별도로 추적 중인
    교차 테이블 SQL 작업이 필요합니다.
agent-set-model-success = 에이전트 { $id }의 모델이 { $value }(으)로 설정되었습니다.
agent-set-model-failed-with-reason = 모델을 설정하지 못했습니다: { $error }
agent-set-no-daemon = 실행 중인 데몬을 찾을 수 없습니다. 다음으로 시작하십시오: librefang start
agent-set-unknown-field = 알 수 없는 필드: { $field }. 지원되는 필드: model
agent-new-no-templates = 에이전트 템플릿을 찾을 수 없습니다
agent-new-no-templates-fix = 에이전트 디렉터리를 설정하려면 `librefang init`을 실행하십시오
agent-new-template-not-found = 템플릿 '{ $name }'을(를) 찾을 수 없습니다
agent-new-template-not-found-fix = 사용 가능한 템플릿을 보려면 `librefang agent new`를 실행하십시오
agent-new-choose-template-prompt =   템플릿을 선택하십시오 [1]: 
agent-sessions-none-active = 활성 세션이 없습니다.
agent-sessions-none-found = 세션을 찾을 수 없습니다.

label-source = 소스
label-name = 이름
label-captured = 캡처됨
label-module = 모듈
label-tools = 도구
label-tags = 태그
label-description = 설명

# daemon.rs
daemon-first-run-setup = 첫 실행이 감지됨 — 빠른 설정을 실행하는 중...
daemon-config-not-found = 구성 파일을 찾을 수 없습니다: { $path }
daemon-config-not-found-fix = ~/.librefang/config.toml에 기본 구성을 생성하려면 `librefang init`을 실행하거나 --config 경로를 확인하십시오.
daemon-log-file-not-found = 로그 파일을 찾을 수 없습니다
daemon-log-file-not-found-fix = 예상 위치: { $path }
daemon-log-not-found-showing-tui = 데몬 로그를 찾을 수 없습니다. { $path }의 TUI 로그를 표시합니다

# hand.rs
hand-install-error-no-toml = 오류: { $path }에서 HAND.toml을 찾을 수 없습니다
hand-install-error-read-toml = { $path } 읽기 오류: { $error }
hand-error-prefix = 오류: { $error }
hand-installed-success = 핸드 설치됨: { $name } ({ $id })
hand-activate-hint = 시작하려면 `librefang hand activate { $id }`를 사용하십시오.
hand-none-available = 사용 가능한 핸드가 없습니다.
hand-list-activate-hint =
    핸드를 활성화하려면 `librefang hand activate <id>`을(를) 사용하십시오.
hand-none-active = 활성 핸드가 없습니다.
label-hand = 핸드
label-instance = 인스턴스
label-agent = 에이전트
hand-status-title = 핸드 상태
label-status-inactive = 비활성
hand-not-found = '{ $id }'에 대한 활성 핸드 또는 설치된 핸드를 찾을 수 없습니다.
hand-activated-success = 핸드 '{ $id }'이(가) 활성화됨 (인스턴스: { $instance }, 에이전트: { $agent })
hand-activate-failed = 핸드 '{ $id }' 활성화에 실패함: { $error }
hand-deactivated-success = 핸드 '{ $id }'이(가) 비활성화됨.
label-failed-reason = 실패함: { $error }
hand-no-active-instance = '{ $id }'에 대한 활성 핸드 인스턴스를 찾을 수 없습니다.
hand-info-not-found = 핸드를 찾을 수 없음: { $error }
hand-no-settings = 핸드 '{ $id }'에 구성 가능한 설정이 없습니다.
hand-settings-title = '{ $id }'의 설정
hand-set-setting-success = 핸드 '{ $id }'에 { $key }={ $value }을(를) 설정함.
hand-reloaded-summary = 핸드 다시 불러옴: { $added }개 추가됨, { $updated }개 업데이트됨, 총 { $total }개.
label-chat-with = 다음과 채팅
hand-chat-quit-hint = (종료하려면 /quit 입력)
hand-chat-prompt-you = 나 >
label-no-response = [응답 없음]
# mcp_cmds.rs
mcp-catalog-unknown-entry = 알 수 없는 MCP 카탈로그 항목: '{ $name }'
mcp-catalog-available-header =
    사용 가능한 MCP 서버 (카탈로그):
mcp-failed-read-config = { $path } 읽기 실패: { $error }
mcp-invalid-toml = { $path }은(는) 유효한 TOML이 아닙니다: { $error }
mcp-already-configured = MCP 서버 '{ $name }'은(는) 이미 구성되어 있습니다. 다시 설치하려면 먼저 `librefang mcp remove { $name }`을(를) 실행하십시오.
mcp-failed-write-config = config.toml 쓰기 실패: { $error }
mcp-add-credentials-hint =
    자격 증명을 추가하려면:
mcp-get-it-here =   여기에서 받으십시오: { $url }
mcp-not-configured = MCP 서버 '{ $name }'이(가) 구성되지 않았습니다
mcp-failed-update-config = config.toml 업데이트 실패: { $error }
mcp-removed-success = { $name }이(가) 제거되었습니다.
mcp-catalog-no-matches = '{ $query }'과(와) 일치하는 MCP 카탈로그 항목이 없습니다.
mcp-catalog-none-available = 사용 가능한 MCP 카탈로그 항목이 없습니다.
mcp-catalog-summary =   카탈로그 항목 { $total }개 (설치됨 { $installed }개)
mcp-catalog-install-hint =   MCP 서버를 설치하려면 `librefang mcp add <id>`을(를) 사용하십시오.
mcp-none-configured = 구성된 MCP 서버가 없습니다.
mcp-list-catalog-hint =   설치 가능한 항목을 나열하려면 `librefang mcp catalog`을(를) 사용하십시오.
mcp-vault-set-hint =   librefang vault set { $name }  # { $help }
mcp-header-name = name
mcp-header-template-id = template_id
mcp-header-transport = transport
mcp-header-details = details

# monitoring.rs
monitoring-audit-reset-destructive = 감사 초기화 재설정은 파괴적입니다 — 진행하려면 `--confirm`과 함께 다시 실행하십시오
monitoring-db-not-found = { $path }에서 데이터베이스를 찾을 수 없습니다
monitoring-db-open-failed = { $path } 열기 실패: { $error }
monitoring-db-truncate-failed = audit_entries 비우기에 실패했습니다: { $error }
monitoring-audit-reset-anchor-deleted = , { $path }의 앵커를 삭제했습니다
monitoring-audit-reset-anchor-none =  (제거할 앵커 파일 없음)
monitoring-audit-reset-success = 감사 추적 초기화 재설정: audit_entries에서 { $count }개 행을 제거했습니다{ $anchor_detail }.
monitoring-audit-reset-would-header =   수행 예정:
monitoring-audit-reset-would-delete =     1. { $path }의 `audit_entries`에서 모든 행 DELETE
monitoring-audit-reset-would-remove-anchor =     2. 앵커 파일 { $path } 제거
monitoring-audit-reset-would-restart =   Merkle 체인은 다음 감사 이벤트부터 다시 시작됩니다.
monitoring-daemon-running-error = 데몬이 { $url }에서 실행 중입니다. 감사 데이터베이스를 건드리지 않습니다
monitoring-daemon-running-error-fix = 먼저 데몬을 중지하십시오: `librefang stop`
monitoring-anchor-remove-failed = 앵커 { $path } 제거에 실패했습니다: { $error }
monitoring-audit-reset-seed-fresh = 다음 데몬 부팅 시 현재 끝점에서 새 Merkle 체인이 시드됩니다.
# skill.rs
skill-install-progress = { $source } 설치 중

# system.rs
migrate-error-home-dir = 오류: 홈 디렉터리를 확인할 수 없습니다
migrate-start-msg = { $source }에서 마이그레이션 중 ({ $path })...
migrate-dry-run-hint =   (테스트 실행 — 변경 사항이 적용되지 않습니다)
migrate-progress-label = 마이그레이션 실행 중
migrate-complete-msg = 마이그레이션 완료
migrate-warn-report-save-failed = 경고: 마이그레이션 보고서를 저장할 수 없습니다: { $error }
migrate-report-saved =
      보고서가 다음 위치에 저장되었습니다: { $path }
migrate-failed-msg = 마이그레이션 실패: { $error }

# maintenance.rs
maintenance-service-install-root-error = root로 실행 중입니다 — 서비스가 사용자 계정이 아닌 root 계정에 설치됩니다. sudo 없이 실행하십시오.
maintenance-service-unsupported = 이 플랫폼에서는 자동 시작 서비스가 지원되지 않습니다.
maintenance-failed-create-dir = { $path } 생성에 실패했습니다: { $error }
maintenance-failed-write-file = { $path } 쓰기에 실패했습니다: { $error }
maintenance-wrote-file = { $path } 작성됨
maintenance-systemctl-reload-failed = systemctl --user daemon-reload 실패
maintenance-service-enabled = 서비스 활성화됨 (다음 로그인 시 시작됩니다)
maintenance-service-start-hint = 지금 시작하려면: systemctl --user start librefang.service
maintenance-service-linger-hint = 헤드리스 서버의 경우 다음도 실행하십시오: loginctl enable-linger
maintenance-systemctl-enable-failed = systemctl --user enable librefang.service 실패
maintenance-launchagent-loaded = LaunchAgent 로드됨 (로그인 시 및 지금 시작됩니다)
maintenance-launchctl-load-failed = launchctl load 실패: { $error }
maintenance-launchctl-run-failed = launchctl 실행 실패: { $error }
maintenance-windows-startup-added = Windows 시작 프로그램에 추가됨 (HKCU\Software\Microsoft\Windows\CurrentVersion\Run)
maintenance-windows-registry-write-failed = 레지스트리 쓰기 실패: { $error }
maintenance-windows-reg-run-failed = reg.exe 실행 실패: { $error }
maintenance-systemd-removed = systemd 사용자 서비스 제거됨
maintenance-systemd-remove-failed = 서비스 파일 제거 실패: { $error }
maintenance-systemd-not-found = systemd 사용자 서비스를 찾을 수 없습니다 — 제거할 항목이 없습니다.
maintenance-launchagent-removed = LaunchAgent 제거됨
maintenance-launchagent-remove-failed = plist 제거 실패: { $error }
maintenance-launchagent-not-found = LaunchAgent를 찾을 수 없습니다 — 제거할 항목이 없습니다.
maintenance-windows-startup-removed = Windows 시작 프로그램에서 제거됨
maintenance-windows-startup-not-found = 시작 프로그램 항목을 찾을 수 없습니다 — 제거할 항목이 없습니다.
maintenance-systemd-status-registered = systemd 사용자 서비스가 등록되어 있습니다
maintenance-status-label-enabled =   활성화됨
maintenance-status-label-active =   활성
maintenance-systemd-status-not-registered = 등록된 systemd 사용자 서비스가 없습니다.
maintenance-service-install-hint = 설정하려면 `librefang service install` 을(를) 실행하십시오.
maintenance-launchagent-status-registered = LaunchAgent가 등록됨
maintenance-status-label-loaded =   로드됨
maintenance-launchagent-status-not-registered = 등록된 LaunchAgent가 없습니다.
maintenance-windows-status-registered = Windows 시작 항목이 등록됨
maintenance-windows-status-not-registered = 등록된 시작 항목이 없습니다.
reset-confirm-message =   { $path }의 모든 데이터를 삭제합니다
      포함: config, 데이터베이스, 에이전트 매니페스트, 자격 증명.
reset-confirm-prompt =   계속하시겠습니까? 확인하려면 'yes'를 입력하십시오: 
reset-not-needed = 초기화 재설정할 항목이 없습니다 — { $path } 이(가) 존재하지 않습니다.
maintenance-update-section = 업데이트
maintenance-update-error-exe-path = 현재 실행 파일 경로를 확인할 수 없습니다: { $error }
maintenance-update-error-check-release = 최신 릴리스를 확인하지 못했습니다: { $error }
maintenance-update-warn-resolve-release = 최신 게시 릴리스를 확인할 수 없습니다: { $error }
maintenance-update-warn-resolve-release-fix = 나중에 다시 시도하거나, 특정 릴리스를 지정하려면 `--version <tag>` 을(를) 전달하십시오.
maintenance-update-available = 더 새로운 게시 릴리스가 있습니다: { $tag }
maintenance-update-run-hint = 설치하려면 `librefang update` 을(를) 실행하십시오.
maintenance-update-same-core = 게시 릴리스 { $tag } 은(는) 현재 바이너리({ $current })와 동일한 CLI 버전 코어를 사용합니다.
maintenance-update-same-core-hint = 이 버전 라인의 최신 게시 빌드를 원하면 `librefang update` 을(를) 실행하십시오.
maintenance-update-ahead = 현재 바이너리 버전 { $current } 은(는) 게시 릴리스 { $tag } 보다 앞서 있습니다.
maintenance-update-compare-unknown = 현재 바이너리를 릴리스 태그 { $tag } 와(과) 비교할 수 없습니다.
maintenance-update-compare-unknown-hint = 정확히 그 릴리스를 원하면 `librefang update --version <tag>` 을(를) 실행하십시오.
maintenance-update-unable-to-determine = 업데이트가 있는지 확인할 수 없습니다.
maintenance-update-unable-to-determine-hint = GitHub Releases에 연결할 수 있을 때 나중에 다시 시도하십시오.
maintenance-update-cannot-compare-safely = 현재 바이너리를 릴리스 태그 { $tag } 와(과) 안전하게 비교할 수 없습니다.
maintenance-update-cannot-compare-safely-hint = 명시적으로 설치하려면 `librefang update --version { $tag }` 으로 다시 실행하십시오.
maintenance-update-windows-daemon-running-error = Windows에서 업데이트하기 전에 실행 중인 데몬을 중지하십시오.
maintenance-update-windows-daemon-running-error-fix = `librefang stop`을 실행한 다음 `librefang update`, 그다음 `librefang start`를 실행하십시오.
maintenance-update-cli-success = LibreFang CLI가 업데이트되었습니다.
maintenance-update-merging-config-defaults = 새 config 기본값을 병합하는 중...
maintenance-update-restart-daemon-hint = 데몬이 실행 중이면 `librefang restart`로 다시 시작하십시오.
maintenance-update-background-launched = 백그라운드에서 업데이트를 시작했습니다.
maintenance-update-background-hint-terminal = 완료된 후 새 터미널을 열고 `librefang --version`을 실행하십시오.
maintenance-update-background-hint-restart = 데몬이 실행 중이면 업데이트가 완료된 후 다시 시작하십시오.
maintenance-update-failed-error = 업데이트 실패: { $error }
maintenance-update-cargo-blocked = 이 바이너리는 cargo로 설치되었습니다. 활성 실행 파일 내부에서 `cargo install`을 실행하는 것은 의도적으로 차단됩니다.
maintenance-update-unofficial-path = 자동 업데이트는 공식 설치 경로({ $path })만 지원합니다. 이 바이너리는 다른 위치에서 실행되고 있습니다.
maintenance-update-package-manager-hint = 이 바이너리가 다른 패키지 관리자에서 설치된 경우 해당 패키지 관리자로 업데이트하십시오.

# doctor_cmd.rs
doctor-check-librefang-dir-ok = LibreFang 디렉터리: { $path }
doctor-check-librefang-dir-fail = LibreFang 디렉터리를 찾을 수 없습니다.
doctor-check-librefang-dir-created = LibreFang 디렉터리를 생성했습니다
doctor-check-librefang-dir-create-fail = 디렉터리 생성에 실패했습니다
doctor-check-librefang-dir-not-found-init = LibreFang 디렉터리를 찾을 수 없습니다. 먼저 `librefang init`을 실행하십시오.
doctor-check-env-ok = .env 파일 (권한 정상)
doctor-check-env-fixed = .env 파일 (권한이 0600으로 수정됨)
doctor-check-env-ok-generic = .env 파일
doctor-check-env-loose-warn = .env 파일의 권한이 느슨합니다({ $mode }). 0600이어야 합니다
doctor-check-env-not-found-warn = .env 파일을 찾을 수 없습니다 (다음으로 생성: librefang config set-key <provider>)
doctor-check-config-ok = Config 파일: { $path }
doctor-check-config-syntax-fail = Config 파일에 구문 오류가 있습니다: { $error }
doctor-check-config-not-found = 구성 파일을 찾을 수 없습니다.
doctor-check-config-created = 기본 config.toml을 생성했습니다
doctor-check-config-create-fail = config.toml 생성에 실패했습니다
doctor-check-cli-version = CLI 버전: { $version } (채널: { $channel })
doctor-check-update-available-warn = 업데이트가 있습니다: { $current } -> { $latest } (https://github.com/librefang/librefang/releases 참조)
doctor-check-cli-up-to-date = CLI가 최신 버전입니다
doctor-check-update-fail-warn = 업데이트를 확인할 수 없습니다 (네트워크 사용 불가)
doctor-check-daemon-running = 데몬이 { $url }에서 실행 중입니다
doctor-check-daemon-not-running-warn = 데몬이 실행 중이 아닙니다 (`librefang start`로 시작하십시오)
doctor-check-port-available = 포트 { $address }을(를) 사용할 수 있습니다
doctor-check-port-in-use-warn = 포트 { $address }이(가) 다른 프로세스에서 사용 중입니다
doctor-check-stale-daemon-json-removed = 오래된 daemon.json을 제거했습니다
doctor-check-stale-daemon-json-warn = 오래된 daemon.json이 발견되었습니다 (데몬이 실행 중이 아님). --repair로 실행하여 정리하십시오.
doctor-check-db-ok = 데이터베이스 파일 (유효한 SQLite)
doctor-check-db-invalid-fail = 데이터베이스 파일이 존재하지만 유효한 SQLite가 아닙니다
doctor-check-db-not-found-warn = 데이터베이스 파일이 없습니다 (첫 실행 시 생성됨)
doctor-check-disk-space-low-warn = 디스크 공간 부족: { $count }MB 사용 가능
doctor-check-disk-space-ok = 디스크 공간: { $count }MB 사용 가능
doctor-check-manifests-ok = 에이전트 매니페스트가 유효합니다
doctor-check-manifest-invalid-fail = 잘못된 매니페스트 { $file }: { $error }
doctor-check-home-dir-fail = 홈 디렉터리를 확인할 수 없습니다
doctor-check-provider-key-rejected-warn = { $name } ({ $env_var }) - 키가 거부되었습니다 (401/403)
doctor-check-endpoint-reachable = { $name } 엔드포인트에 연결할 수 있습니다 ({ $endpoint })
doctor-check-endpoint-unreachable-warn = { $name } 엔드포인트에 연결할 수 없습니다 ({ $endpoint })
doctor-check-channel-token-format-warn = { $name } ({ $env_var }) - 예상치 못한 토큰 형식
doctor-check-config-env-missing-warn = 구성이 { $env_var }을(를) 참조하지만 env 또는 .env에 설정되어 있지 않습니다
doctor-check-config-deser-ok = 구성이 KernelConfig로 역직렬화됩니다
doctor-check-exec-policy = Exec 정책: mode={ $mode }, safe_bins={ $count }
doctor-check-include-file-ok = 포함 파일: { $path }
doctor-check-include-file-missing-warn = 포함 파일 누락: { $path }
doctor-check-include-file-not-found-fail = 포함 파일을 찾을 수 없음: { $path }
doctor-check-mcp-servers-count = 구성된 MCP 서버: { $count }
doctor-check-mcp-empty-command-warn = MCP 서버 '{ $name }'의 command가 비어 있습니다
doctor-check-mcp-empty-url-warn = MCP 서버 '{ $name }'의 URL이 비어 있습니다
doctor-check-mcp-empty-base-url-warn = MCP 서버 '{ $name }'의 base_url이 비어 있습니다
doctor-check-mcp-no-compat-tools-warn = MCP 서버 '{ $name }'에 구성된 http_compat 도구가 없습니다
doctor-check-mcp-compat-header-empty-name-warn = MCP 서버 '{ $name }'에 name이 비어 있는 http_compat 헤더가 있습니다
doctor-check-mcp-compat-header-no-value-warn = MCP 서버 '{ $name }'에 value/value_env가 없는 http_compat 헤더가 있습니다
doctor-check-mcp-compat-tool-empty-name-warn = MCP 서버 '{ $name }'에 name이 비어 있는 http_compat 도구가 있습니다
doctor-check-mcp-compat-tool-empty-path-warn = MCP 서버 '{ $name }'에 path가 비어 있는 http_compat 도구가 있습니다
doctor-check-config-deser-fail = 구성의 KernelConfig 역직렬화에 실패했습니다: { $error }
doctor-check-skills-loaded = 로드된 스킬: { $count }
doctor-check-skills-load-fail-warn = 스킬 로드 실패: { $error }
doctor-check-skills-injection-ok = 모든 스킬이 프롬프트 인젝션 검사를 통과했습니다
doctor-check-mcp-catalog-templates = MCP 카탈로그 템플릿: { $templates }
doctor-check-mcp-configured-servers = 구성된 MCP 서버: { $configured }
doctor-check-running-agents = 실행 중인 에이전트: { $count }
doctor-check-daemon-uptime = 데몬 가동 시간: { $hours }시간 { $mins }분
doctor-check-db-connectivity-ok = 데이터베이스 연결: OK
doctor-check-db-status-fail = 데이터베이스 상태: { $status }
doctor-check-health-detail-status-warn = 상태 세부 정보가 { $status }을(를) 반환했습니다
doctor-check-health-detail-fail-warn = 데몬 상태 조회에 실패했습니다: { $error }
doctor-check-skills-loaded-daemon = 데몬에 로드된 스킬: { $count }
doctor-check-rust-version = Rust: { $version }
doctor-check-rust-not-found-fail = Rust 툴체인을 찾을 수 없습니다
doctor-check-python-version = Python: { $version }
doctor-check-python-not-found-warn = Python을 찾을 수 없습니다 (Python 스킬 런타임에 필요)
doctor-check-node-version = Node.js: { $version }
doctor-check-node-not-found-warn = Node.js를 찾을 수 없습니다 (Node 스킬 런타임에 필요)
doctor-prompt-create-dir =     지금 생성하시겠습니까? [Y/n] 
doctor-prompt-create-config =     기본 구성을 생성하시겠습니까? [Y/n] 
doctor-section-providers =   LLM 제공자:
doctor-section-connectivity = 

  네트워크 연결:
doctor-section-channels = 

  채널 통합:
doctor-section-config-val = 

  구성 검증:
doctor-section-skills = 

  스킬:
doctor-check-skills-injection-warn = 스킬에 프롬프트 인젝션 경고: { $name }
doctor-section-mcp-servers =
  MCP 서버:
doctor-section-daemon-health =
  데몬 상태:
doctor-check-daemon-mcp-status = MCP 서버: { $configured }개 구성됨, { $connected }개 연결됨
doctor-check-daemon-mcp-health = MCP 서버 상태: { $healthy }/{ $total } 정상

doctor-suggest-groq = https://console.groq.com       (무료, 빠름)
doctor-suggest-gemini = https://aistudio.google.com    (무료 등급)
doctor-suggest-deepseek = https://platform.deepseek.com  (저비용)

desktop-install-launched = 데스크톱 앱을 실행했습니다.
desktop-install-launch-fail = { $path } 실행에 실패했습니다: { $error }
desktop-install-launch-fail-generic = 데스크톱 앱 실행에 실패했습니다: { $error }
desktop-install-not-installed = LibreFang Desktop이 설치되어 있지 않습니다.
desktop-install-prompt =   지금 다운로드하여 설치하시겠습니까? [Y/n] 
desktop-install-skipped = 건너뛰었습니다. 나중에 설치할 수 있습니다:
desktop-install-skipped-brew =   brew install --cask librefang   (macOS)
desktop-install-skipped-manual =   또는 https://github.com/librefang/librefang/releases 에서 다운로드하십시오
desktop-install-fetching = 최신 릴리스 정보를 가져오는 중...
desktop-install-unsupported = 데스크톱 자동 설치를 지원하지 않는 플랫폼입니다.
desktop-install-download-manual = 수동으로 다운로드하십시오: https://github.com/librefang/librefang/releases
desktop-install-github-fail = GitHub에 연결하지 못했습니다: { $error }
desktop-install-parse-fail = 릴리스 정보를 파싱하지 못했습니다: { $error }
desktop-install-kv-asset = 에셋
desktop-install-downloading = 다운로드 중...
desktop-install-download-fail = 다운로드에 실패했습니다: { $error }
desktop-install-download-complete = 다운로드 완료.
desktop-install-installing = 설치 중...
desktop-install-success = LibreFang Desktop가 성공적으로 설치되었습니다.
desktop-install-fail = 설치에 실패했습니다: { $error }
desktop-install-running-installer = 설치 프로그램을 실행하는 중...

doctor-audit-vault-key-unset = LIBREFANG_VAULT_KEY가 설정되지 않음 — 볼트 암호화가 비활성화됨.
doctor-audit-vault-key-invalid-base64 = LIBREFANG_VAULT_KEY가 유효한 base64가 아닙니다: { $error }
doctor-audit-vault-key-invalid-base64-hint = 다음으로 생성하십시오: openssl rand -base64 32
doctor-audit-vault-key-wrong-length = LIBREFANG_VAULT_KEY가 { $count }바이트로 디코딩됩니다. 정확히 32바이트여야 합니다. base64 디코딩 후 ASCII 32자는 32바이트가 아니라는 점에 유의하십시오.
doctor-audit-vault-key-wrong-length-hint = 새 32바이트 키를 생성하십시오: openssl rand -base64 32 (44자 출력)
doctor-audit-vault-key-ok = LIBREFANG_VAULT_KEY가 32바이트로 디코딩됩니다.

doctor-audit-api-listen-no-config = config.toml을 찾을 수 없음 — api_listen 검사를 건너뜁니다.
doctor-audit-api-listen-invalid-toml = config.toml이 유효한 TOML이 아닙니다: { $error }
doctor-audit-api-listen-invalid-toml-hint = ~/.librefang/config.toml을 편집하거나 `librefang doctor --repair`를 실행하십시오.
doctor-audit-api-listen-unset = config에 api_listen이 설정되지 않았습니다 — 커널이 기본값을 사용합니다.
doctor-audit-api-listen-invalid-addr = api_listen `{ $address }`이(가) 유효한 소켓 주소가 아닙니다: { $error }
doctor-audit-api-listen-invalid-addr-hint = `host:port` 형식을 사용하십시오. 예: `127.0.0.1:4545` 또는 `[::1]:4545`.
doctor-audit-api-listen-port-zero = api_listen `{ $address }`이(가) 포트 0(OS 할당 임시 포트)을 사용합니다. 바인딩 후 클라이언트가 데몬 URL을 찾을 수 없습니다.
doctor-audit-api-listen-port-zero-hint = 명시적인 포트를 선택하십시오(기본값 4545). 예: `127.0.0.1:4545`.
doctor-audit-api-listen-privileged = api_listen 포트 { $port }은(는) 특권 포트(<1024)입니다. 데몬이 root 없이 바인딩에 실패합니다.
doctor-audit-api-listen-privileged-hint = 의도적으로 root가 필요한 경우가 아니라면 1024 이상의 포트를 사용하십시오(기본값 4545).
doctor-audit-api-listen-ok = api_listen `{ $address }`이(가) 정상적으로 파싱됩니다.

doctor-audit-config-not-found = { $path }이(가) 존재하지 않습니다.
doctor-audit-config-not-found-hint = 기본 config를 생성하려면 `librefang init`을 실행하십시오.
doctor-audit-config-read-fail = { $path } 읽기에 실패했습니다: { $error }
doctor-audit-config-ok = { $path }이(가) TOML로 파싱됩니다.
doctor-audit-config-syntax-error = { $path }에 TOML 구문 오류가 있습니다: { $error }
doctor-audit-config-syntax-error-hint = { $path }을(를) 편집하거나 백업에서 복원하십시오.

# launcher menu items
launcher-menu-get-started = 시작하기
launcher-menu-get-started-hint = 공급자, API 키, 모델, 마이그레이션
launcher-menu-settings = 설정
launcher-menu-settings-hint = 공급자, API 키, 모델, 라우팅
launcher-menu-chat = 에이전트와 채팅
launcher-menu-chat-hint = 터미널에서 빠른 채팅
launcher-menu-dashboard = 대시보드 열기
launcher-menu-dashboard-hint = 브라우저에서 웹 UI 실행
launcher-menu-desktop = 데스크톱 앱 열기
launcher-menu-desktop-hint = 네이티브 데스크톱 앱 실행
launcher-menu-tui = 터미널 UI 실행
launcher-menu-tui-hint = 완전한 대화형 TUI 대시보드
launcher-menu-help = 모든 명령 표시
launcher-menu-help-hint = 전체 --help 출력 표시

# launcher screen strings
launcher-welcome = 환영합니다! 설정을 시작하겠습니다.
launcher-checking-daemon = 데몬 확인 중…
launcher-daemon-running = { $url }에서 데몬 실행 중
launcher-daemon-agents = { $count ->
    [one]  ({ $count }개 에이전트)
   *[other]  ({ $count }개 에이전트)
}
launcher-daemon-no-running = 실행 중인 데몬 없음
launcher-provider = 제공자: { $provider }
launcher-no-keys = 감지된 API 키 없음
launcher-hint-re-run =   제공자를 구성하려면 'Re-run setup'을 실행하십시오
launcher-hint-get-started =   구성하려면 'Get started'를 선택하십시오
launcher-migration-question = { $source }에서 오셨나요? 
launcher-migration-hint = 'Get started'에는 자동 마이그레이션이 포함됩니다.
launcher-menu-hints = ↑↓/jk 탐색  1-9 빠른 선택  enter 확인  q 종료
launcher-help-title = 모든 명령
launcher-help-subtitle =   — q/Esc 뒤로
launcher-help-hints = ↑↓/jk 스크롤  PgUp/PgDn  g/G 맨 위/맨 아래  q 뒤로

# CLI shared UI strings
common-warning-config-default = 경고: { $error }; 이 명령에는 기본 config 값을 사용합니다
ui-brand-tagline = 오픈 소스 에이전트 운영 체제
ui-brand-title = LibreFang Agent OS
ui-label-hint = 힌트:
ui-label-next-steps = 다음 단계
ui-label-fix = 수정:
ui-label-try = 시도:
ui-provider-not-set = { $env_var }이(가) 설정되지 않음
progress-fail = [FAIL]

# Table headers / Shared labels
label-header-name = 이름
label-header-kind = 종류
label-header-configured = 구성됨
label-header-token = 토큰
label-header-alias = 별칭
label-header-provider = 공급자
label-header-id = ID
label-header-agent = 에이전트
label-header-category = 카테고리
label-header-description = 설명
label-header-hand = 핸드
label-header-instance = 인스턴스
label-header-model = 모델
label-header-status = 상태
label-header-type = 유형
label-header-timestamp = 타임스탬프
label-header-event = 이벤트
label-header-key = 키
label-header-value = 값
label-header-enabled = 활성화됨
label-header-url = URL

# Channel command specific keys
channel-header-msgs-24h = 24시간 메시지
channel-error-save-failed-no-body = 저장 실패 (오류 본문 없음)

# Models command specific keys
model-none-in-catalog = 카탈로그에 모델이 없습니다.
model-header-model = 모델
model-header-tier = 등급
model-header-context = 컨텍스트
model-header-resolves-to = 확인 대상
model-header-auth = 인증
model-header-models = 모델
model-header-base-url = BASE URL
model-picker-item =     { $idx }. { $id } { $tier }

# Approvals command specific keys
approval-none-pending = 대기 중인 승인이 없습니다.
approval-header-request = 요청

# Auth command specific keys
auth-error-create-home-dir = LibreFang 홈 디렉터리 생성에 실패했습니다: { $error }
auth-error-write-secrets = secrets.env 쓰기에 실패했습니다: { $error }
auth-error-parse-config = config.toml 파싱에 실패했습니다: { $error }
auth-error-default-model-not-table = default_model이 테이블이 아닙니다
auth-error-write-config = config.toml 쓰기에 실패했습니다: { $error }
auth-pool-add-hint = 다음으로 추가하십시오:
auth-pool-add-example =   librefang auth pool add openai OPENAI_API_KEY_1 --label Primary --priority 10
auth-pool-header = { $provider }  ({ $strategy })
auth-pool-keys-available =   키: { $available }/{ $total } 사용 가능
auth-pool-cooldown-left = ({ $secs }초 남음)
auth-pool-status-invalid = 유효하지 않음
auth-pool-status-exhausted = 소진됨
auth-pool-status-cooldown = 쿨다운
auth-pool-status-env-missing = env-missing
auth-pool-status-healthy = 정상
auth-pool-key-requests = requests={ $count }
auth-pool-key-item =     - [{ $label }] { $key_display }  priority={ $pri }{ $reqs_str }  status={ $status }
auth-hash-add-config-hint = config.toml에 추가하십시오:
auth-hash-config-entry =   dashboard_pass_hash = "{ $hash }"

# Agent command specific keys
agent-spawn-id-label =   ID:   { $id }
agent-spawn-name-label =   이름: { $name }
error-unknown = 알 수 없는 오류
label-unknown = <알 수 없음>
label-header-state = 상태
label-header-created = 생성일
label-header-msgs = 메시지
label-header-last-active = 마지막 활동
label-session-state-running = 실행 중
label-session-state-idle = 유휴

# Daemon command specific keys
daemon-error-resolve-exe = 현재 실행 파일을 확인하지 못함: { $error }
daemon-error-create-log-dir = 로그 디렉터리 { $path } 생성 실패: { $error }
daemon-error-open-log = 데몬 로그 { $path } 열기 실패: { $error }
daemon-error-clone-log-handle = 데몬 로그 핸들 { $path } 복제 실패: { $error }
daemon-error-spawn-detached = 분리된 데몬 생성: { $error }
daemon-error-failed-create-log-dir = 로그 디렉터리 { $path } 생성 실패: { $error }
daemon-error-failed-open-log = 데몬 로그 파일 { $path } 열기 실패: { $error }

# --- Skill commands ---
skill-name-empty = 스킬 이름이 비어 있습니다
skill-name-unsafe = 안전하지 않은 스킬 이름 '{ $name }': 단일 경로 구성 요소여야 합니다 ('/', '\', '..' 또는 절대 경로 불가)
skill-hand-not-found = Hand '{ $hand }'을(를) { $path }에서 찾을 수 없습니다
skill-openclaw-detected = OpenClaw 스킬 형식을 감지했습니다. 변환 중...
skill-install-refused = 스킬 설치를 거부합니다: { $error }
skill-write-manifest-failed = 매니페스트 쓰기 실패: { $error }
skill-openclaw-installed-to-hand = OpenClaw 스킬 '{ $name }'을(를) hand '{ $hand }'에 설치했습니다
skill-openclaw-installed = OpenClaw 스킬 설치됨: { $name }
skill-openclaw-convert-failed = OpenClaw 스킬 변환 실패: { $error }
skill-no-toml = { $path }에서 skill.toml을 찾을 수 없습니다
skill-read-toml-failed = skill.toml 읽기 오류: { $error }
skill-parse-toml-failed = skill.toml 파싱 오류: { $error }
skill-installed-to-hand = 스킬 '{ $name }' v{ $version }을(를) hand '{ $hand }'에 설치했습니다
skill-installed = 스킬 설치됨: { $name } v{ $version }
skill-installed-hub-to-hand = { $source } { $version }을(를) hand '{ $hand }'에 설치했습니다
skill-installed-hub = { $source } { $version } 설치됨
skill-install-failed = 스킬 설치 실패: { $error }
skill-list-none-hand = hand '{ $hand }'에 설치된 스킬이 없습니다.
skill-list-none = 설치된 스킬이 없습니다.
skill-list-count-hand = hand '{ $hand }'에 { $count }개의 스킬이 설치되어 있습니다:
skill-list-count = { $count }개의 스킬이 설치되어 있습니다:
skill-list-load-failed = 스킬 로드 중 오류: { $error }
skill-removed-from-hand = 핸드 '{ $hand }'에서 스킬 '{ $name }'을(를) 제거했습니다
skill-removed = 제거된 스킬: { $name }
skill-remove-failed = 스킬 제거 실패: { $error }
skill-search-none = "{ $query }"에 대한 스킬을 찾을 수 없습니다.
skill-search-results-header = "{ $query }"와(과) 일치하는 스킬:
skill-search-failed = 검색 실패: { $error }
skill-validation-failed = 스킬 검증 실패: { $error }
skill-execution-failed = 스킬 실행 실패: { $error }
skill-package-failed = 스킬 패키징 실패: { $error }
skill-determine-dir-failed = 현재 디렉터리를 확인할 수 없습니다: { $error }
skill-unsupported-runtime = 지원되지 않는 런타임 '{ $runtime }'. 다음 중 하나를 선택하십시오: python, node, wasm.
skill-create-dir-failed = 스킬 디렉터리 생성 중 오류: { $error }
skill-registry-load-failed = 스킬 레지스트리 로드 중 오류: { $error }
skill-not-found = { $path }에서 스킬 '{ $name }'을(를) 찾을 수 없습니다
skill-read-file-failed = { $path } 읽기 실패: { $error }
skill-create-skills-dir-failed = skills 디렉터리 생성 실패: { $error }
skill-create-failed = 생성 실패: { $error }
skill-update-failed = 업데이트 실패: { $error }
skill-patch-failed = 패치 실패: { $error }
skill-delete-failed = 삭제 실패: { $error }
skill-rollback-failed = 롤백 실패: { $error }
skill-write-file-failed = 파일 쓰기 실패: { $error }
skill-remove-file-failed = 파일 제거 실패: { $error }
skill-serialize-history-failed = 기록 직렬화에 실패했습니다: { $error }
skill-read-pending-failed = 대기 디렉터리 읽기에 실패했습니다: { $error }
skill-load-candidate-failed = 후보 로드에 실패했습니다: { $error }
skill-render-candidate-failed = 후보를 TOML로 렌더링하는 데 실패했습니다: { $error }
skill-approve-candidate-failed = 승인에 실패했습니다: { $error }
skill-reject-candidate-failed = 거부에 실패했습니다: { $error }
skill-publish-failed = 게시에 실패했습니다: { $error }
skill-evolution-label = 스킬: { $name }
skill-version-label = 현재 버전: { $version }
skill-use-count-label = 사용 횟수: { $count }
skill-evolution-count-label = 진화 횟수: { $count }
skill-no-history = 기록된 버전 이력이 없습니다.
skill-no-pending = 대기 중인 스킬 후보가 없습니다.{ $filter }
skill-pending-filter =  (필터: 에이전트 { $agent })
skill-approved-candidate = 후보 { $id }을(를) 승인하여 스킬 '{ $name }'(v{ $version })을(를) 설치했습니다.
skill-rejected-candidate = 후보 { $id }을(를) 거부하고 제거했습니다.
skill-validated = 검증된 스킬: { $name } v{ $version }
skill-validated-runtime =   런타임: { $runtime }
skill-validated-source =   소스: { $path }
skill-validated-description =   설명: { $description }
skill-validated-tools =   도구: { $tools }
skill-refusing-warnings = 심각한 검증 경고가 있는 스킬의 실행을 거부합니다.
skill-validated-only = 검증 전용: 실행할 도구가 선언되지 않았습니다.
skill-invalid-input-json = 잘못된 --input JSON: { $error }
skill-tool-result-header = 도구 결과 ({ $name }):
skill-validation-complete = 검증이 완료되었습니다.
skill-execution-skipped = 실행을 건너뛰었습니다: { $message }
skill-preparing = 스킬 준비 중: { $name } v{ $version }
skill-refusing-publish = 심각한 검증 경고가 있는 스킬은 게시를 거부합니다.
skill-bundle-created = 번들이 생성되었습니다: { $path }
skill-bundle-sha =   SHA256: { $sha }
skill-bundle-size =   크기: { $size } bytes
skill-dry-run = 테스트 실행만 수행합니다.
skill-dry-run-repo =   저장소: { $repo }
skill-dry-run-tag =   태그: { $tag }
skill-github-token-required = 게시하려면 GITHUB_TOKEN 또는 GH_TOKEN을 설정하거나 --dry-run으로 다시 실행하십시오.
skill-publishing-progress = { $name }@{ $tag } 게시 중
skill-publish-success = { $name }을(를) { $repo }@{ $tag }에 게시했습니다
skill-publish-release-url = 릴리스: { $url }
skill-warnings-none =   경고: 없음
skill-warnings-header =   경고:
skill-prompt-name = 스킬 이름: 
skill-prompt-description = 설명: 
skill-prompt-runtime = 런타임 (python/node/wasm) [python]: 
skill-created = 스킬이 생성되었습니다: { $path }
skill-created-files-header = 파일:
skill-created-next-steps-header = 다음 단계:
skill-created-step-edit =   { $step }. 진입점을 편집하여 스킬 로직을 구현하십시오
skill-created-step-test =   { $step }. 로컬에서 테스트: librefang skill test { $path }
skill-created-step-install =   { $step }. 설치: librefang skill install { $path }

# --- Monitoring & Status ---
monitoring-no-audit = 감사 항목이 없습니다.
monitoring-no-memory = 에이전트 '{ $agent }'의 메모리 항목이 없습니다.
monitoring-no-devices = 페어링된 기기가 없습니다.
monitoring-no-webhooks = 구성된 웹훅이 없습니다.
label-last-seen = 마지막 확인
status-watch-header =   { $status } ({ $interval }초마다 새로고침, Ctrl+C로 종료)
status-warning-config = 경고: { $error }; 상태 표시에 기본 구성 값을 사용합니다
status-summary-up = librefang { $version } { $state } uptime={ $uptime } { $auth } ({ $base })
status-peers-connected = { $connected }개 연결됨 / { $total }개 알려짐
status-agents-active = { $active }개 실행 중 / 총 { $total }개
status-mb = { $mb } MB
status-summary-down = librefang 중지됨 home={ $home } default={ $provider }/{ $model }
status-uptime-format = { $hours }시간 { $mins }분
# --- Brand/proper names ---
brand-openai = OpenAI
brand-openrouter = OpenRouter
brand-deepseek = DeepSeek
brand-deepinfra = DeepInfra
brand-byteplus = BytePlus
brand-azure-openai = Azure OpenAI
brand-github-copilot = GitHub Copilot
brand-huggingface = Hugging Face
brand-openai-codex = OpenAI Codex
brand-claude-code = Claude Code
brand-vertex-ai = Vertex AI
brand-nvidia-nim = NVIDIA NIM
brand-zai = Z.ai
brand-kimi-coding = Kimi Coding
brand-alibaba-coding-plan = Alibaba Coding Plan
brand-slack-app = Slack App
brand-slack-bot = Slack Bot
brand-telegram = Telegram
brand-discord = Discord
brand-openclaw-openfang = OpenClaw / OpenFang
brand-openclaw = OpenClaw
brand-openfang = OpenFang

# --- Number/unit formatting templates ---
format-bytes-gib = { $value } GiB
format-bytes-mib = { $value } MiB
format-bytes-kib = { $value } KiB
format-bytes-b = { $value } B
format-size-mb = ({ $value } MB)

format-uptime-s = { $secs }초
format-uptime-ms = { $mins }분 { $secs }초
format-uptime-hm = { $hours }시간 { $mins }분
format-uptime-hms = { $hours }시간 { $mins }분 { $secs }초
format-uptime-dh = { $days }일 { $hours }시간
format-uptime-dhm = { $days }일 { $hours }시간 { $mins }분

# --- Desktop install & Update errors ---
desktop-install-unsupported-platform = 지원되지 않는 플랫폼
desktop-install-error-hdiutil-attach = hdiutil attach 실패: { $error }
desktop-install-error-app-not-found = DMG에서 LibreFang.app을 찾을 수 없습니다
desktop-install-error-remove-old = 이전 설치 제거 실패: { $error }
desktop-install-error-cp = cp 실패: { $error }
desktop-install-error-copy-applications = /Applications로 복사 실패: { $error }
desktop-install-error-run-installer = 설치 프로그램 실행 실패: { $error }
desktop-install-error-installer-status = 설치 프로그램이 다음 코드로 종료됨: { $status }
desktop-install-error-localappdata = %LOCALAPPDATA%를 확인할 수 없습니다
desktop-install-error-binary-not-found = 설치는 완료되었으나 예상 위치에서 바이너리를 찾을 수 없습니다
desktop-install-error-home-dir = 홈 디렉터리를 확인할 수 없습니다
desktop-install-error-create-dir = { $path } 생성 실패: { $error }
desktop-install-error-copy-appimage = AppImage 복사 실패: { $error }
desktop-install-error-http = HTTP 요청 실패: { $error }
desktop-install-error-create = { $path }을(를) 생성할 수 없습니다: { $error }
desktop-install-error-write = 쓰기 오류: { $error }

maintenance-error-github-request = GitHub 요청 실패: { $error }
maintenance-error-github-status = GitHub API가 { $status }을(를) 반환했습니다
maintenance-error-decode-release = 릴리스 메타데이터 디코딩 실패: { $error }
maintenance-error-missing-tag = 릴리스 메타데이터에 `tag_name`이 없습니다
maintenance-error-decode-list = 릴리스 목록 디코딩 실패: { $error }
maintenance-error-no-release = '{ $channel }' 채널에 일치하는 릴리스를 찾을 수 없습니다
maintenance-error-http-client = HTTP 클라이언트 빌드 실패: { $error }
maintenance-error-powershell-updater = PowerShell 업데이터 실행 실패: { $error }
maintenance-error-run-installer = 설치 프로그램을 실행하지 못했습니다: { $error }
maintenance-error-installer-status = 설치 프로그램이 상태 { $status }(으)로 종료되었습니다
maintenance-error-download-fail = 다운로드에 실패했습니다: { $error }
maintenance-error-download-status = 다운로드가 { $status }을(를) 반환했습니다
maintenance-error-read-response = 응답 본문을 읽지 못했습니다: { $error }
maintenance-error-create-dir = 업데이터 디렉터리를 만들지 못했습니다: { $error }
maintenance-error-create-script = 업데이터 스크립트를 만들지 못했습니다: { $error }
maintenance-error-write-script = 업데이터 스크립트를 쓰지 못했습니다: { $error }

common-error-find-exe = 실행 파일을 찾을 수 없습니다: { $error }
common-error-spawn-daemon = 데몬을 생성하지 못했습니다: { $error }
common-error-daemon-timeout = 데몬이 10초 이내에 준비되지 않았습니다

# tui/chat_runner.rs
chat-runner-owner-notice = [owner_notice] { $preview }
chat-runner-error-prefix = 오류: { $error }
chat-runner-no-active-connection = 활성 연결이 없습니다
chat-runner-unknown-command = 알 수 없는 명령: { $command }. /help을 입력하십시오
chat-runner-status-mode-daemon = 모드: 데몬 ({ $url })
chat-runner-status-agent = 에이전트: { $name }
chat-runner-status-mode-inprocess = 모드: 인프로세스
chat-runner-status-agents-count = 에이전트: { $count }
chat-runner-status-mode-disconnected = 모드: 연결 끊김
chat-runner-chat-history-cleared = 채팅 기록을 지웠습니다.
chat-runner-session-reset = 세션 초기화 재설정 — 새로 시작합니다.
chat-runner-session-reset-failed = 세션을 초기화 재설정할 수 없습니다.
chat-runner-agent-killed = 에이전트 "{ $name }"을(를) 종료했습니다.
chat-runner-failed-kill-agent = 에이전트 "{ $name }"을(를) 종료하지 못했습니다.
chat-runner-kill-failed = 종료 실패: { $error }
chat-runner-no-backend-connected = 연결된 백엔드가 없습니다.
chat-runner-no-models-available = 사용 가능한 모델이 없습니다.
chat-runner-switched-model = { $model }(으)로 전환했습니다
chat-runner-failed-switch-model = { $model }(으)로 전환하지 못했습니다
chat-runner-switch-failed = 전환 실패: { $error }
chat-runner-welcome-help-hint = /help 명령 목록 • /exit 종료
chat-runner-spawning-agent = '{ $name }' 에이전트 생성 중…
chat-runner-no-agent-templates = 에이전트 템플릿을 찾을 수 없습니다. `librefang init`을 실행하십시오.
chat-runner-invalid-template = 잘못된 템플릿 '{ $name }': { $error }
chat-runner-spawn-failed = 생성 실패: { $error }
chat-runner-booting-kernel = 커널 부팅 중…
chat-runner-booting-kernel-hint =   커널이 초기화되는 동안 잠시 걸릴 수 있습니다.
chat-runner-failed-start = 시작하지 못했습니다
chat-runner-press-esc-to-exit =   Esc를 눌러 종료하십시오.

# tui/event.rs
tui-event-workflow-completed = 워크플로가 완료되었습니다
tui-event-workflow-exec-not-available-in-process = 인프로세스 모드에서는 워크플로 실행을 사용할 수 없습니다
tui-event-workflow-create-not-available-in-process = 인프로세스 모드에서는 워크플로 생성을 사용할 수 없습니다
tui-event-trigger-create-not-available-in-process = 인프로세스 모드에서는 트리거 생성을 사용할 수 없습니다
tui-event-trigger-delete-failed = 트리거 { $trigger_id }을(를) 삭제하지 못했습니다
tui-event-trigger-delete-not-available-in-process = 인프로세스 모드에서는 트리거 삭제를 사용할 수 없습니다
tui-event-agent-kill-failed = 에이전트 { $agent_id }을(를) 종료하지 못했습니다
tui-event-agent-invalid-id = 잘못된 에이전트 ID: { $agent_id }
tui-event-skills-fetch-failed = 스킬을 가져오지 못했습니다
tui-event-mcp-fetch-failed = MCP 서버를 가져오지 못했습니다
tui-event-skills-update-failed = 스킬을 업데이트하지 못했습니다
tui-event-skills-update-error = 스킬 업데이트: { $error }
tui-event-mcp-update-failed = MCP 서버를 업데이트하지 못했습니다
tui-event-mcp-update-error = MCP 업데이트: { $error }
tui-event-session-delete-failed = 세션 { $session_id }을(를) 삭제하지 못했습니다
tui-event-session-management-not-available-in-process = 인프로세스 모드에서는 세션 관리를 사용할 수 없습니다
tui-event-kv-save-failed = KV 쌍을 저장하지 못했습니다
tui-event-kv-not-available-in-process = 인프로세스 모드에서는 메모리 KV를 사용할 수 없습니다
tui-event-kv-delete-failed = KV 쌍을 삭제하지 못했습니다
tui-event-skill-install-failed = { $slug }을(를) 설치하지 못했습니다
tui-event-skill-install-not-available-in-process = 인프로세스 모드에서는 스킬 설치를 사용할 수 없습니다
tui-event-skill-uninstall-failed = { $name }을(를) 제거하지 못했습니다
tui-event-skill-uninstall-not-available-in-process = 인프로세스 모드에서는 스킬 제거를 사용할 수 없습니다
tui-event-security-verification-complete = 검증 완료
tui-event-security-chain-not-applicable = 인프로세스 모드: 체인을 적용할 수 없습니다
tui-event-provider-save-key-failed = { $name }의 키를 저장하지 못했습니다
tui-event-provider-key-management-not-available-in-process = 인프로세스 모드에서는 공급자 키 관리를 사용할 수 없습니다
tui-event-provider-delete-key-failed = { $name }의 키를 삭제하지 못했습니다
tui-event-provider-connection-ok = 연결 정상
tui-event-provider-test-failed = 테스트 실패
tui-event-provider-test-not-available-in-process = 인프로세스 모드에서는 공급자 테스트를 사용할 수 없습니다
tui-event-hand-activation-failed = 활성화 실패
tui-event-hand-activate-failed-error = 활성화 실패: { $error }
tui-event-hand-activation-failed-error = 활성화 실패: { $error }
tui-event-hand-deactivate-failed = { $instance_id } 비활성화 실패
tui-event-hand-deactivate-failed-error = 비활성화 실패: { $error }
tui-event-hand-invalid-instance-id = 잘못된 인스턴스 ID: { $error }
tui-event-hand-pause-failed = { $instance_id } 일시 중지 실패
tui-event-hand-pause-failed-error = 일시 중지 실패: { $error }
tui-event-hand-resume-failed = { $instance_id } 재개 실패
tui-event-hand-resume-failed-error = 재개 실패: { $error }
tui-event-extension-install-failed = { $id } 설치 실패
tui-event-extension-install-failed-error = 설치 실패: { $error }
tui-event-extension-install-not-supported = 인프로세스 모드에서는 설치를 지원하지 않습니다 — CLI를 사용하십시오
tui-event-extension-remove-failed = { $id } 제거 실패
tui-event-extension-remove-not-supported = 인프로세스 모드에서는 제거를 지원하지 않습니다 — CLI를 사용하십시오
tui-event-extension-reconnect-failed = { $id } 재연결 실패
tui-event-extension-reconnect-not-supported = 인프로세스 모드에서는 재연결을 지원하지 않습니다
tui-event-comms-message-sent = 메시지를 보냈습니다
tui-event-comms-send-failed = 전송 실패
tui-event-comms-send-not-supported-in-process = 인프로세스에서는 전송을 지원하지 않습니다
tui-event-comms-task-posted = 작업을 게시했습니다
tui-event-comms-post-failed = 게시 실패
tui-event-comms-post-not-supported-in-process = 인프로세스에서는 작업 게시를 지원하지 않습니다
tui-event-stream-runtime-error = 런타임 오류: { $error }
tui-event-stream-connection-failed = 연결 실패: { $error }
tui-event-agent-spawn-failed-fallback = 에이전트 생성에 실패했습니다

# tui/mod.rs
tui-mod-session-deleted = 세션 { $id }이(가) 삭제되었습니다.
tui-mod-saved-key = 저장된 키: { $key }
tui-mod-deleted-key = 삭제된 키: { $key }
tui-mod-skill-installed = 설치됨: { $name }
tui-mod-skill-uninstalled = 제거됨: { $name }
tui-mod-key-saved-for = { $name }에 대한 키가 저장되었습니다
tui-mod-key-deleted-for = { $name }에 대한 키가 삭제되었습니다
tui-mod-hand-activated = 활성화됨: { $name }
tui-mod-hand-deactivated = 비활성화됨: { $id }
tui-mod-hand-paused = Hand가 일시 중지됨
tui-mod-hand-resumed = Hand가 재개됨
tui-mod-extension-installed = 설치됨: { $id }
tui-mod-extension-removed = 제거됨: { $id }
tui-mod-extension-reconnected = { $id } 재연결됨: 도구 { $tools }개
tui-mod-no-agents-running = 실행 중인 에이전트가 없습니다.
tui-mod-agent-killed = 에이전트 "{ $name }"이(가) 종료되었습니다.
tui-mod-failed-kill-agent = 에이전트 "{ $name }" 종료에 실패했습니다.
tui-mod-invalid-manifest = 잘못된 매니페스트: { $error }
tui-mod-spawn-failed = 생성 실패: { $error }
tui-mod-help-help = /help         — 이 도움말 표시
tui-mod-help-model = /model        — 모델 선택기 열기 (Ctrl+M)
tui-mod-help-model-arg = /model <name> — 모델로 바로 전환
tui-mod-help-status = /status       — 연결 및 에이전트 정보
tui-mod-help-agents = /agents       — 실행 중인 에이전트 목록
tui-mod-help-clear = /clear        — 채팅 기록 지우기
tui-mod-help-new = /new          — 세션 초기화 (새로 시작)
tui-mod-help-kill = /kill         — 현재 에이전트 종료
tui-mod-help-exit = /exit         — 채팅 세션 종료
tui-mod-status-mode-daemon = 모드: 데몬 ({ $url })
tui-mod-status-agent = 에이전트: { $name }
tui-mod-status-mode-inprocess = 모드: 인프로세스
tui-mod-status-agents-count = 에이전트: { $count }
tui-mod-status-mode-disconnected = 모드: 연결 끊김
tui-mod-chat-history-cleared = 채팅 기록을 지웠습니다.
tui-mod-session-reset = 세션 초기화됨 — 새로 시작합니다.
tui-mod-session-reset-failed = 세션을 초기화할 수 없습니다.
tui-mod-available-hands = 사용 가능한 hands ({ $count }):
tui-mod-active-hands = 활성 hands ({ $count }):
tui-mod-hands-info-requires-inprocess = Hands 정보는 인프로세스 모드가 필요합니다. 대신 Hands 탭을 사용하십시오.
tui-mod-unknown-command = 알 수 없는 명령: { $command }. /help 를 입력하십시오
tui-mod-error-symbol =  ✘ { $error }
tui-mod-press-ctrl-c-again-to-quit = 종료하려면 Ctrl+C 를 다시 누르십시오
tui-mod-ctrl-c-status-bar = Ctrl+C×2 종료  Tab/Ctrl+←→ 전환
tui-mod-trigger-deleted = 트리거 { $id } 삭제됨.
tui-mod-agent-killed-status = 에이전트 { $id } 종료됨.
tui-mod-agent-kill-failed = 종료 실패: { $error }
tui-mod-agent-skills-updated = 에이전트 { $id }의 스킬이 업데이트됨.
tui-mod-agent-mcp-updated = 에이전트 { $id }의 MCP 서버가 업데이트되었습니다.
tui-mod-ready = 준비됨
tui-mod-setup = 설정
tui-mod-workflow-created = 워크플로가 생성되었습니다!
tui-mod-trigger-created = 트리거가 생성되었습니다!
tui-tab-dashboard = 대시보드
tui-tab-agents = 에이전트
tui-tab-chat = 채팅
tui-tab-sessions = 세션
tui-tab-workflows = 플로우
tui-tab-triggers = 트리거
tui-tab-memory = 메모리
tui-tab-skills = 스킬
tui-tab-hands = 핸드
tui-tab-extensions = 확장
tui-tab-templates = 템플릿
tui-tab-peers = 피어
tui-tab-comms = 통신
tui-tab-security = 보안
tui-tab-audit = 감사
tui-tab-usage = 사용량
tui-tab-settings = 구성
tui-tab-logs = 로그
# welcome.rs
tui-welcome-menu-connect = 데몬에 연결
tui-welcome-menu-connect-hint = API로 실행 중인 에이전트와 대화
tui-welcome-menu-chat = 빠른 채팅
tui-welcome-menu-chat-hint = 로컬에서 커널 부팅, 데몬 불필요
tui-welcome-menu-setup = 설정 마법사
tui-welcome-menu-setup-hint = 프로바이더 및 채널 구성
tui-welcome-menu-exit = 종료
tui-welcome-menu-exit-hint = LibreFang 종료
tui-welcome-tagline = 에이전트 운영체제
tui-welcome-ctrl-c-quit = 종료하려면 Ctrl+C를 다시 누르십시오
tui-welcome-hint-bar = ↑↓ 탐색  enter 선택  q 종료
tui-welcome-checking-daemon = 데몬 확인 중…
tui-welcome-agent-count =
    { $count ->
        [one]  • { $count }개 에이전트
       *[other]  • { $count }개 에이전트
    }
tui-welcome-daemon-status = 데몬 { $url }
tui-welcome-no-daemon = 실행 중인 데몬 없음
tui-welcome-provider = 프로바이더: { $provider }
tui-welcome-no-api-keys = API 키 없음
tui-welcome-run-hint-prefix =  — 실행 
tui-welcome-setup-complete = 설정 완료!

# sessions.rs
tui-sessions-title = 세션
tui-sessions-filter = (필터: "{ $query }")
tui-sessions-count =
    { $count ->
        [one] 세션 1개
       *[other] 세션 { $count }개
    }
tui-sessions-header-agent = 에이전트
tui-sessions-header-id = 세션 ID
tui-sessions-header-msgs = 메시지
tui-sessions-header-created = 생성됨
tui-sessions-loading = 세션 로드 중…
tui-sessions-empty = 아직 세션이 없습니다. 채팅을 시작하여 세션을 생성하십시오.
tui-sessions-delete-confirm = 이 세션을 삭제하시겠습니까? [y] 예  [any] 취소
tui-sessions-hints = ↑↓ 탐색  Enter 열기  d 삭제  / 검색  r 새로고침

# peers.rs
tui-peers-title = 피어
tui-peers-network = OFP 피어 네트워크
tui-peers-count =
    { $count ->
        [one] 피어 1개
       *[other] 피어 { $count }개
    }
tui-peers-header-node-id = 노드 ID
tui-peers-header-name = 이름
tui-peers-header-address = 주소
tui-peers-header-status = 상태
tui-peers-header-agents = 에이전트
tui-peers-header-protocol = 프로토콜
tui-peers-status-active = 활성
tui-peers-status-offline = 오프라인
tui-peers-status-pending = 대기 중
tui-peers-loading = 피어를 검색하는 중…
tui-peers-empty = 연결된 피어가 없습니다. config.toml에서 OFP 네트워킹을 활성화하십시오.
tui-peers-hints = ↑↓ 탐색  r 새로고침  (15초마다 자동 새로고침)

# usage.rs
tui-usage-title = 사용량
tui-usage-hints = [1] 요약  [2] 모델별  [3] 에이전트별  [r] 새로고침
tui-usage-tab-summary = 1 요약
tui-usage-tab-model = 2 모델별
tui-usage-tab-agent = 3 에이전트별
tui-usage-loading = 사용량 데이터를 불러오는 중…
tui-usage-loading-simple = 불러오는 중…
tui-usage-card-input = 입력 토큰
tui-usage-card-output = 출력 토큰
tui-usage-card-cost = 총 비용
tui-usage-card-calls = API 호출
tui-usage-header-model = 모델
tui-usage-header-input = 입력 토큰
tui-usage-header-output = 출력 토큰
tui-usage-header-cost = 비용
tui-usage-header-calls = 호출
tui-usage-header-agent = 에이전트
tui-usage-header-total-tokens = 총 토큰
tui-usage-header-tool-calls = 도구 호출
tui-usage-empty = 사용량 데이터가 없습니다. 메시지를 보내 토큰 통계를 확인하십시오.

# hands.rs
tui-hands-title = 핸드
tui-hands-tab-marketplace = 마켓플레이스
tui-hands-tab-active = 활성
tui-hands-loading = Hands 로드 중…
tui-hands-loading-active = 활성 Hands 로드 중…
tui-hands-empty-marketplace = 로드된 Hand 정의가 없습니다.
tui-hands-empty-active = 활성 Hands가 없습니다. [1]을 눌러 마켓플레이스를 탐색하십시오.
tui-hands-status-ready = 준비됨
tui-hands-status-setup = 설정
tui-hands-status-active = 활성
tui-hands-status-paused = 일시 중지됨
tui-hands-status-unknown = 알 수 없음
tui-hands-hints-marketplace =   [↑↓] 탐색  [a/Enter] 활성화  [r] 새로고침
tui-hands-hints-active =   [↑↓] 탐색  [p] 일시 중지/재개  [d] 비활성화  [r] 새로고침
tui-hands-confirm-deactivate =   이 Hand를 비활성화하시겠습니까? [y] 예  [any] 취소
tui-hands-header-name = 이름
tui-hands-header-category = 카테고리
tui-hands-header-status = 상태
tui-hands-header-description = 설명
tui-hands-header-agent = 에이전트
tui-hands-header-hand = 핸드
tui-hands-header-since = 시작 시각
tui-hands-category-content = 콘텐츠
tui-hands-category-security = 보안
tui-hands-category-development = 개발
tui-hands-category-productivity = 생산성

# logs.rs
tui-logs-title = 로그
tui-logs-badge-auto = 자동
tui-logs-badge-paused = 일시 중지됨
tui-logs-label-level = 레벨
tui-logs-filter-all = 전체
tui-logs-filter-error = 오류
tui-logs-filter-warn = 경고
tui-logs-filter-info = 정보
tui-logs-filter-active =   │ 필터: "{ $query }"
tui-logs-entries-count =   │ { $count }개 항목
tui-logs-header-timestamp = 타임스탬프
tui-logs-header-level = 수준
tui-logs-header-action = 작업
tui-logs-header-agent = 에이전트
tui-logs-header-detail = 세부 정보
tui-logs-loading = 로그 로딩 중…
tui-logs-empty = 로그 항목이 없습니다. 데몬을 시작하면 로그가 표시됩니다.
tui-logs-hints =   [↑↓] 탐색  [f] 수준 필터  [/] 검색  [a] 자동 새로고침  [r] 새로고침

# security.rs
tui-security-title = 보안
tui-security-active-features =   { $active }/{ $total }개 기능 활성
tui-security-sections-sub =   │  코어 · 구성 가능 · 모니터링
tui-security-section-core = 코어 보안
tui-security-section-configurable = 구성 가능
tui-security-section-monitoring = 모니터링
tui-security-header-feature = 기능
tui-security-header-status = 상태
tui-security-header-description = 설명
tui-security-status-active = 활성
tui-security-status-inactive = 비활성
tui-security-verifying = 감사 체인 검증 중…
tui-security-verify-prompt = [v]를 눌러 감사 체인 무결성을 검증하십시오
tui-security-verify-success = 감사 체인 검증됨
tui-security-verify-failed = 감사 체인 검증 실패
tui-security-hints =   [↑↓] 스크롤  [v] 체인 검증  [r] 새로고침
tui-security-feat-path-traversal-name = 경로 탐색 방지
tui-security-feat-path-traversal-desc = safe_resolve_path가 ../../ 공격을 차단합니다
tui-security-feat-ssrf-name = SSRF 방어
tui-security-feat-ssrf-desc = HTTP 가져오기에서 사설 IP와 메타데이터 엔드포인트를 차단합니다
tui-security-feat-subprocess-name = 하위 프로세스 격리
tui-security-feat-subprocess-desc = 자식 프로세스에 env_clear() + 선택적 변수 적용
tui-security-feat-wasm-name = WASM 이중 계측
tui-security-feat-wasm-desc = 워치독 스레드를 사용한 Fuel + epoch 인터럽트
tui-security-feat-capability-name = 기능 상속
tui-security-feat-capability-desc = validate_capability_inheritance가 권한 상승을 방지합니다
tui-security-feat-secret-name = 시크릿 제로화
tui-security-feat-secret-desc = Zeroizing<String>이 메모리에서 API 키를 자동으로 지웁니다
tui-security-feat-ed25519-name = Ed25519 매니페스트 서명
tui-security-feat-ed25519-desc = Ed25519 검증을 사용한 서명된 에이전트 매니페스트
tui-security-feat-taint-name = 오염 추적
tui-security-feat-taint-desc = 도구 경계를 넘는 정보 흐름 추적
tui-security-feat-ofp-name = OFP 와이어 인증
tui-security-feat-ofp-desc = nonce를 사용한 HMAC-SHA256 상호 인증
tui-security-feat-rbac-name = RBAC 다중 사용자
tui-security-feat-rbac-desc = 사용자 계층 구조를 갖춘 역할 기반 접근 제어
tui-security-feat-rate-name = 속도 제한
tui-security-feat-rate-desc = 비용 인식 토큰을 사용한 GCRA 속도 제한기
tui-security-feat-headers-name = 보안 헤더
tui-security-feat-headers-desc = CSP, X-Frame-Options, HSTS 미들웨어
tui-security-feat-merkle-name = Merkle 감사 추적
tui-security-feat-merkle-desc = 변조 탐지 기능이 있는 해시 체인 감사 로그
tui-security-feat-heartbeat-name = 하트비트 모니터
tui-security-feat-heartbeat-desc = 재시작 제한이 있는 백그라운드 상태 검사
tui-security-feat-prompt-name = 프롬프트 인젝션 스캐너
tui-security-feat-prompt-desc = 재정의 시도 및 데이터 유출을 탐지

# templates.rs
tui-templates-title = 템플릿
tui-templates-cat-all = 전체
tui-templates-cat-general = 일반
tui-templates-cat-development = 개발
tui-templates-cat-research = 리서치
tui-templates-cat-writing = 작문
tui-templates-cat-business = 비즈니스
tui-templates-header-template = 템플릿
tui-templates-header-category = 카테고리
tui-templates-header-provider-model = 프로바이더/모델
tui-templates-header-description = 설명
tui-templates-loading = 템플릿 로드 중…
tui-templates-empty = 사용 가능한 템플릿이 없습니다.
tui-templates-detail-provider =   프로바이더: { $provider }/{ $model }  
tui-templates-hints =   [↑↓] 탐색  [Enter] 에이전트 생성  [f] 카테고리 필터  [r] 새로고침
tui-templates-provider-not-configured = 프로바이더 '{ $provider }'이(가) 구성되지 않았습니다. 먼저 설정에서 API 키를 설정하십시오.
tui-templates-name-general-assistant = 일반 어시스턴트
tui-templates-desc-general-assistant = 일상 작업을 위한 다재다능한 AI 어시스턴트
tui-templates-name-code-helper = 코드 도우미
tui-templates-desc-code-helper = 코드 리뷰 및 디버깅을 지원하는 프로그래밍 어시스턴트
tui-templates-name-researcher = 리서처
tui-templates-desc-researcher = 웹 검색을 통한 심층 연구 및 분석
tui-templates-name-writer = 작가
tui-templates-desc-writer = 창작 및 기술 문서 작성 어시스턴트
tui-templates-name-data-analyst = 데이터 분석가
tui-templates-desc-data-analyst = 데이터 분석, 시각화 및 SQL 쿼리
tui-templates-name-devops-engineer = DevOps 엔지니어
tui-templates-desc-devops-engineer = 인프라, CI/CD 및 배포 지원
tui-templates-name-customer-support = 고객 지원
tui-templates-desc-customer-support = 전문 고객 서비스 에이전트
tui-templates-name-tutor = 튜터
tui-templates-desc-tutor = 어떤 주제든 학습을 돕는 인내심 있는 교육 어시스턴트
tui-templates-name-api-designer = API 설계자
tui-templates-desc-api-designer = REST/GraphQL API 설계 및 문서화
tui-templates-name-meeting-notes = 회의록
tui-templates-desc-meeting-notes = 회의 전사, 요약 및 작업 항목

# audit.rs
tui-audit-title = 감사 추적
tui-audit-filter-all = 전체
tui-audit-filter-spawn = 에이전트 생성됨
tui-audit-filter-kill = 에이전트 종료됨
tui-audit-filter-tool = 도구 사용됨
tui-audit-filter-network = 네트워크
tui-audit-filter-shell = 셸 실행
tui-audit-action-spawn = 에이전트 생성됨
tui-audit-action-kill = 에이전트 종료됨
tui-audit-action-tool = 도구 사용됨
tui-audit-action-network = 네트워크 접근
tui-audit-action-shell = 셸 실행
tui-audit-action-denied = 접근 거부됨
tui-audit-action-config = 구성 변경됨
tui-audit-label-filter = 필터:
tui-audit-entries-count = { $count }개 항목
tui-audit-header-timestamp = 타임스탬프
tui-audit-header-action = 작업
tui-audit-header-agent = 에이전트
tui-audit-header-hash = 해시
tui-audit-header-detail = 상세
tui-audit-loading = 감사 추적을 불러오는 중…
tui-audit-empty = 아직 감사 항목이 없습니다. 에이전트 작업이 여기에 표시됩니다.
tui-audit-chain-unverified = 체인: 검증되지 않음
tui-audit-chain-verified = 체인: 검증됨
tui-audit-chain-failed = 체인: 검증 실패
tui-audit-hints =   [↑↓] 탐색  [f] 필터  [v] 체인 검증  [r] 새로고침

# dashboard.rs
tui-dashboard-title = 대시보드
tui-dashboard-hints =   [r] 새로고침  [a] 에이전트  [↑↓] 스크롤  [PgUp/PgDn] 빠른 스크롤
tui-dashboard-dreams-title = DREAMS
tui-dashboard-auto-dream-enabled = Auto-Dream 활성화됨
tui-dashboard-auto-dream-disabled = Auto-Dream 비활성화됨
tui-dashboard-dream-details = phase={ $phase }  tools={ $tools }  mems={ $mems }
tui-dashboard-stat-agents = 에이전트
tui-dashboard-stat-uptime = 가동 시간
tui-dashboard-stat-provider = 공급자
tui-dashboard-stat-model = 모델
tui-dashboard-audit-time = 시간
tui-dashboard-audit-agent = 에이전트
tui-dashboard-audit-action = 작업
tui-dashboard-audit-detail = 세부 정보
tui-dashboard-loading = 로딩 중…
tui-dashboard-no-audit = 아직 감사 추적 항목이 없습니다.

# comms.rs
tui-comms-title = 통신
tui-comms-tab-topology = 토폴로지 (에이전트 { $agents }개, 엣지 { $edges }개)
tui-comms-tab-events = 이벤트 ({ $count })
tui-comms-hints =   [s]전송  [t]작업  [r]새로고침  [Tab] 포커스  [↑↓] 스크롤
tui-comms-loading = 토폴로지 로딩 중…
tui-comms-empty = 실행 중인 에이전트가 없습니다. 통신을 보려면 에이전트를 시작하십시오.
tui-comms-events-empty = 아직 에이전트 간 이벤트가 없습니다. 활동이 여기에 표시됩니다.
tui-comms-modal-send-title =  메시지 전송 
tui-comms-modal-send-from = 보낸 곳 (에이전트 ID):
tui-comms-modal-send-to = 받는 곳 (에이전트 ID):
tui-comms-modal-send-msg = 메시지:
tui-comms-modal-send-hints = [Tab] 필드  [Enter] 전송  [Esc] 취소
tui-comms-modal-task-title =  작업 게시 
tui-comms-modal-task-title-field = 제목:
tui-comms-modal-task-desc = 설명:
tui-comms-modal-task-assign = 할당 대상 (agent ID, 선택):
tui-comms-modal-task-hints = [Tab] 필드  [Enter] 게시  [Esc] 취소

# settings.rs
tui-settings-title = 설정
tui-settings-hints-input =   [Enter] 저장  [Esc] 취소
tui-settings-hints-providers =   [↑↓] 탐색  [e] 키 설정  [d] 키 삭제  [t] 테스트  [r] 새로고침
tui-settings-hints-models =   [↑↓] 탐색  [r] 새로고침
tui-settings-hints-tools =   [↑↓] 탐색  [r] 새로고침
tui-settings-tab-providers = 1 공급자
tui-settings-tab-models = 2 모델
tui-settings-tab-tools = 3 도구
tui-settings-providers-header-provider = 공급자
tui-settings-providers-header-status = 상태
tui-settings-providers-header-env = 환경 변수
tui-settings-providers-loading = 공급자 로드 중…
tui-settings-providers-empty = 구성된 공급자가 없습니다. `librefang init`을 실행하여 설정하십시오.
tui-settings-providers-status-online = 온라인 ({ $ms }ms)
tui-settings-providers-status-offline = 오프라인
tui-settings-providers-status-local = 로컬
tui-settings-providers-status-configured = 구성됨
tui-settings-providers-status-notset = 설정 안 됨
tui-settings-providers-input-prompt = { $provider }의 API 키를 입력하십시오: 
tui-settings-providers-latency = 지연 시간: { $ms }ms
tui-settings-models-header-id = 모델 ID
tui-settings-models-header-provider = 제공자
tui-settings-models-header-tier = 등급
tui-settings-models-header-context = 컨텍스트
tui-settings-models-header-cost = 비용 (1M당 입력/출력)
tui-settings-models-loading = 모델 로드 중…
tui-settings-models-empty = 사용 가능한 모델이 없습니다.
tui-settings-tools-header-name = 도구 이름
tui-settings-tools-header-desc = 설명
tui-settings-tools-empty = 사용 가능한 도구가 없습니다.
# chat.rs
tui-chat-input-staged =   ({ $count }개 추가 대기)
tui-chat-hints-modelpicker =     [↑↓] 탐색  [Enter] 선택  [Esc] 닫기  [type] 필터
tui-chat-hints-streaming =     [Enter] 추가 대기  [↑↓] 스크롤  [Esc] 중지
tui-chat-hints-history =     [Enter] 전송  [↑↓] 기록  [PgUp/PgDn] 스크롤  [Esc] 뒤로
tui-chat-hints-normal =     [Enter] 전송  [Ctrl+M] 모델  [↑↓] 기록  [PgUp/PgDn] 스크롤  [Esc] 뒤로
tui-chat-modelpicker-title =  모델 전환 
tui-chat-modelpicker-empty = 일치하는 모델 없음
tui-chat-welcome-ready = 채팅 준비됨
tui-chat-welcome-suggest =   이렇게 물어보세요:
tui-chat-welcome-q1 = "이 코드베이스를 설명해줘"
tui-chat-welcome-q2 = "...에 대한 단위 테스트를 작성해줘"
tui-chat-welcome-q3 = "이 함수는 무엇을 하나요?"
tui-chat-welcome-footer =   /help를 입력하면 명령 목록  •  Ctrl+M으로 모델 전환
tui-chat-tokens-estimated =   ~{ $count } 토큰
tui-chat-tokens-detail =   [토큰: 입력 { $in } / 출력 { $out }{ $cost }]
tui-chat-tool-input = 입력: 
tui-chat-tool-error = 오류: 
tui-chat-tool-result = 결과: 
tui-chat-tool-running = 실행 중…
tui-chat-thinking = 생각 중…
tui-chat-mode-daemon = 데몬
tui-chat-mode-inprocess = 인프로세스

# free_provider_guide.rs
tui-guide-hint-groq = 무료 등급, 매우 빠른 추론
tui-guide-hint-gemini = 무료 등급, 넉넉한 할당량 (Google 계정)
tui-guide-hint-deepseek = 신규 계정에 500만 무료 토큰
tui-guide-label-apikey =  API 키 
tui-guide-warn-env = .env: { $error }

# init_wizard.rs
tui-init-welcome-tagline = 에이전트 운영 체제
tui-init-welcome-sec1 = 샌드박스 실행, WASM 격리, SSRF 보호
tui-init-welcome-sec2 = 서명된 매니페스트, 감사 추적, 오염 추적
tui-init-welcome-sec3 = RBAC, 기능 검사, 비밀 정보 제로화
tui-init-welcome-sec4 = API 키는 절대 기록되지 않으며, 0600 파일 권한 적용
tui-init-welcome-resp1 = 에이전트는 코드를 실행하고, 네트워크에 접근하며,
tui-init-welcome-resp2 = 사용자를 대신해 작동할 수 있습니다.
tui-init-welcome-resp-warn = 에이전트의 행동에 대한 책임은 사용자에게 있습니다.
tui-init-welcome-hints =   [Enter] 이해했습니다    [Esc] 취소
tui-init-migrate-checking =   기존 설치를 확인하는 중...
tui-init-migrate-openfang-detected =   OpenFang 설치 감지됨
tui-init-migrate-openclaw-detected =   OpenClaw 설치 감지됨
tui-init-migrate-openfang-summary = OpenFang 구성 및 데이터
tui-init-migrate-openclaw-agents = { $count }개 에이전트 ({ $names })
tui-init-migrate-openclaw-no-agents = 에이전트 없음
tui-init-migrate-openclaw-channels = { $count }개 채널 ({ $names })
tui-init-migrate-openclaw-no-channels = 채널 없음
tui-init-migrate-openclaw-skills = { $count }개 스킬
tui-init-migrate-openclaw-no-skills = 스킬 없음
tui-init-migrate-openclaw-memory = 메모리 파일
tui-init-migrate-openclaw-no-memory = 메모리 파일 없음
tui-init-migrate-openclaw-config = 구성
tui-init-migrate-opt-yes = 예
tui-init-migrate-opt-yes-desc = 설정 및 데이터 마이그레이션
tui-init-migrate-opt-no = 아니요
tui-init-migrate-opt-no-desc = 새로 시작
tui-init-migrate-hints =   [↑↓] 탐색  [Enter] 선택  [Esc] 건너뛰기
tui-init-migrate-running-openfang =  OpenFang에서 마이그레이션하는 중...
tui-init-migrate-running-openclaw =  OpenClaw에서 마이그레이션하는 중...
tui-init-migrate-done-failed = 마이그레이션 실패: { $error }
tui-init-migrate-done-config = 구성 마이그레이션됨
tui-init-migrate-done-agents = { $count }개 에이전트 가져옴 ({ $names })
tui-init-migrate-done-channels = { $count }개 채널 ({ $names })
tui-init-migrate-done-memory = 메모리 파일이 복사됨
tui-init-migrate-done-skills = { $count }개 스킬을 가져옴
tui-init-migrate-done-sessions = { $count }개 세션을 가져옴
tui-init-migrate-done-skipped = { $name } 건너뜀 ({ $reason })
tui-init-migrate-done-summary =   { $imported }개 가져옴, { $skipped }개 건너뜀, { $warnings }개 경고
tui-init-migrate-done-continue =   [Enter] 계속  
tui-init-migrate-done-autoadvancing = (자동 진행 중...)
tui-init-provider-prompt =   LLM 공급자를 선택하십시오:
tui-init-provider-cli-detected = CLI 감지됨
tui-init-provider-no-key-needed = API 키 필요 없음
tui-init-provider-local-no-key = 로컬, 키 필요 없음
tui-init-provider-requires-with-hint = { $env_var } 필요 ({ $hint })
tui-init-provider-requires = { $env_var } 필요
tui-init-provider-hints =   [↑↓/jk] 탐색  [Enter] 선택  [Esc] 취소
tui-init-hint-freetier = 무료 등급
tui-init-hint-cheap = 저렴
tui-init-hint-fast = 빠른 추론
tui-init-hint-pat = PAT 사용
tui-init-hint-nokey = API 키 없음
tui-init-hint-local = 로컬
tui-init-apikey-prompt =   { $provider } API 키를 입력하십시오:
tui-init-apikey-env-hint =     또는 { $env_var } 환경 변수를 설정하십시오
tui-init-apikey-testing =  API 키 테스트 중...
tui-init-apikey-verified = API 키 검증됨
tui-init-apikey-saved =     ~/.librefang/.env에 저장됨
tui-init-apikey-verify-failed = 검증할 수 없음 (작동할 수도 있음)
tui-init-apikey-save-failed = 검증되었으나 .env 저장에 실패
tui-init-apikey-save-failed-hints =     [Enter] 저장 재시도  ·  [Esc] 키 편집  (키는 이미 검증됨 — 디스크에 저장된 것 없음)
tui-init-apikey-hints =   [Enter] 확인  [Esc] 뒤로
tui-init-model-prompt =   { $provider }의 기본 모델을 선택하십시오:
tui-init-model-hints =   [↑↓/jk] 탐색  [Enter] 선택  [Esc] 뒤로    * = 기본값
tui-init-routing-title =   스마트 모델 라우팅
tui-init-routing-desc1 =   작업 복잡도에 따라 적절한 모델을 자동으로 선택합니다.
tui-init-routing-desc2 =   간단한 작업은 저렴하고 빠른 모델을, 복잡한 작업은
tui-init-routing-desc3 =   프런티어 모델을 사용합니다. 품질 저하 없이 비용을 절감합니다.
tui-init-routing-opt-yes = 예
tui-init-routing-opt-yes-desc = 3개 모델 선택 (빠름 / 균형 / 프런티어)
tui-init-routing-opt-no = 아니요
tui-init-routing-opt-no-desc = 모든 작업에 하나의 모델 사용
tui-init-routing-hints =   [↑↓] 탐색  [Enter] 선택  [Esc] 뒤로
tui-init-routing-pick-hints =   [↑↓/jk] 탐색  [Enter] 선택  [Esc] 뒤로
tui-init-routing-tier-fast = 빠름
tui-init-routing-tier-balanced = 균형
tui-init-routing-tier-frontier = 프런티어
tui-init-routing-tier-fast-desc = 빠른 조회, 인사, 간단한 Q&A
tui-init-routing-tier-balanced-desc = 일반 대화, 일반 작업
tui-init-routing-tier-frontier-desc = 다단계 추론, 코드 생성
tui-init-complete-success-daemon = 설정 완료 — 데몬 실행 중
tui-init-complete-success = 설정 완료!
tui-init-complete-label-provider =   제공자:    
tui-init-complete-label-model =   모델:       
tui-init-complete-label-daemon =   데몬:      
tui-init-complete-daemon-running = { $url }에서 실행 중
tui-init-complete-daemon-not-running = 실행되지 않음
tui-init-complete-daemon-pending = 대기 중
tui-init-complete-question =   LibreFang을 어떻게 사용하시겠습니까?
tui-init-complete-desktop-desc-installed = 시스템 트레이가 있는 네이티브 창
tui-init-complete-desktop-desc-not-installed = 설치되지 않음
tui-init-complete-opt-desktop = 데스크톱 앱
tui-init-complete-opt-desktop-badge = (권장)
tui-init-complete-opt-dashboard = 웹 대시보드
tui-init-complete-opt-dashboard-desc = 기본 브라우저에서 열기
tui-init-complete-opt-chat = 터미널 채팅
tui-init-complete-opt-chat-desc = 바로 여기에서 대화형 채팅
tui-init-complete-hints =   [↑↓/jk] 탐색  [Enter] 실행  [1/2/3] 빠른 선택
tui-init-step-label = { $total } 중 { $current }
tui-init-complete-err-no-provider = 선택된 제공자가 없습니다
tui-init-complete-err-home-dir = 홈 디렉터리를 확인할 수 없습니다
tui-init-complete-err-write-config = config 작성 실패: { $error }
tui-init-complete-err-daemon-failed = 데몬 실패: { $error }
tui-init-routing-pick-prefix = 선택
tui-init-routing-pick-suffix = 모델 ({ $step }/3):
tui-init-complete-setup-prefix = 설정 완료 — 

# agents.rs
tui-agents-tool-file-read-desc = 파일 읽기
tui-agents-tool-file-write-desc = 파일 쓰기
tui-agents-tool-file-list-desc = 디렉터리 내용 나열
tui-agents-tool-memory-store-desc = 에이전트 메모리에 데이터 저장
tui-agents-tool-memory-recall-desc = 메모리에서 데이터 회수
tui-agents-tool-memory-list-desc = 저장된 모든 메모리 키 나열
tui-agents-tool-web-fetch-desc = 웹 페이지 가져오기
tui-agents-tool-shell-exec-desc = 셸 명령 실행
tui-agents-tool-agent-send-desc = 다른 에이전트에 메시지 전송
tui-agents-tool-agent-list-desc = 실행 중인 에이전트 나열

tui-agents-title-create-method = 에이전트 생성
tui-agents-title-templates = 템플릿
tui-agents-title-custom-name = 사용자 지정 — 이름
tui-agents-title-custom-desc = 사용자 지정 — 설명
tui-agents-title-custom-prompt = 사용자 지정 — 시스템 프롬프트
tui-agents-title-custom-tools = 사용자 지정 — 도구
tui-agents-title-custom-skills = 사용자 지정 — 스킬
tui-agents-title-custom-mcp = 사용자 지정 — MCP 서버
tui-agents-title-spawning = 생성 중...
tui-agents-title-screen = 에이전트
tui-agents-title-detail = 에이전트 상세

tui-agents-prompt-create-method =   에이전트를 어떻게 생성하시겠습니까?
tui-agents-prompt-name = 에이전트 이름:
tui-agents-prompt-desc = 설명:
tui-agents-prompt-prompt = 시스템 프롬프트:
tui-agents-prompt-tools =   도구 선택 (Space로 전환):
tui-agents-prompt-skills =   스킬 선택 (선택 없음 = 모든 스킬):
tui-agents-prompt-mcp =   MCP 서버 선택 (선택 없음 = 모든 서버):
tui-agents-prompt-edit-skills =   Space로 전환, Enter로 저장 (선택 없음 = 전체):
tui-agents-prompt-spawning =   에이전트 생성 중...
tui-agents-label-no-agent-selected = 선택된 에이전트 없음.
tui-agents-label-none-available = (사용 가능 없음)

tui-agents-opt-templates =   템플릿에서 선택
tui-agents-opt-templates-desc =   (사전 구축된 에이전트)
tui-agents-opt-custom =   사용자 지정 에이전트 구축
tui-agents-opt-custom-desc =   (이름, 도구, 프롬프트 선택)

tui-agents-header-state = 상태
tui-agents-header-name = 이름
tui-agents-header-model = 모델
tui-agents-header-id = ID
tui-agents-opt-create-new = 새 에이전트 생성

tui-agents-hints-filter =   [입력] 필터  [Enter] 적용  [Esc] 검색 취소
tui-agents-hints-list =   [↑↓] 탐색  [Enter] 상세  [/] 검색  [Esc] 뒤로
tui-agents-hints-detail =   [s] 스킬 편집  [m] MCP 편집  [c] 채팅  [k] 종료  [Esc] 뒤로
tui-agents-hints-navigate =     [↑↓] 탐색  [Enter] 선택  [Esc] 뒤로
tui-agents-hints-input =     [Enter] 다음  [Esc] 뒤로
tui-agents-hints-tools =     [↑↓] 탐색  [Space] 전환  [Enter] 생성  [Esc] 뒤로
tui-agents-hints-skills =     [↑↓] 탐색  [Space] 전환  [Enter] 다음  [Esc] 뒤로
tui-agents-hints-mcp =     [↑↓] 탐색  [Space] 전환  [Enter] 생성  [Esc] 뒤로
tui-agents-hints-save =     [↑↓] 탐색  [Space] 전환  [Enter] 저장  [Esc] 취소

tui-agents-placeholder-name = my-agent
tui-agents-placeholder-desc = 사용자 지정 에이전트
tui-agents-placeholder-prompt = 당신은 유용한 에이전트입니다.
tui-agents-label-placeholder =     placeholder: { $placeholder }

tui-agents-detail-id =   ID:       
tui-agents-detail-name =   이름:      
tui-agents-detail-state =   상태:      
tui-agents-detail-provider =   공급자:     
tui-agents-detail-model =   모델:      
tui-agents-detail-created =   생성:      
tui-agents-detail-active =   활성:      
tui-agents-detail-tags =   태그:      
tui-agents-detail-caps =   권한:      
tui-agents-detail-parent =   상위:      
tui-agents-detail-children =   하위:      
tui-agents-detail-skills =   스킬:      
tui-agents-detail-mcp =   MCP:      
tui-agents-detail-all-skills = [모든 스킬]
tui-agents-detail-all-servers = [모든 서버]
tui-agents-detail-none = [없음]
tui-agents-default-desc = 사용자 지정 { $name } 에이전트
tui-agents-default-prompt = 당신은 유용한 에이전트인 { $name }입니다.

# --- Workflows screen ---
tui-workflows-title-screen = 워크플로
tui-workflows-header-id = ID
tui-workflows-header-name = 이름
tui-workflows-header-steps = 단계
tui-workflows-header-created = 생성됨
tui-workflows-loading = 워크플로 로드 중...
tui-workflows-empty-state = 정의된 워크플로 없음. [n]으로 생성하십시오.
tui-workflows-create-new-option =   + 새 워크플로 생성
tui-workflows-hints-list =   [↑↓] 탐색  [Enter] 실행 보기  [x] 실행  [n] 새로 만들기  [r] 새로고침
tui-workflows-title-runs = 실행 내역: { $name }
tui-workflows-header-run-id = 실행 ID
tui-workflows-header-state = 상태
tui-workflows-header-duration = 소요 시간
tui-workflows-header-output = 출력
tui-workflows-runs-empty = 아직 실행 없음. 목록에서 [x]를 눌러 실행하십시오.
tui-workflows-hints-runs =   [↑↓] 탐색  [r] 새로고침  [Esc] 뒤로
tui-workflows-title-create = 새 워크플로 생성
tui-workflows-create-step =   { $current } / { $total } 단계
tui-workflows-label-name = 워크플로 이름:
tui-workflows-placeholder-name = my-workflow
tui-workflows-label-desc = 설명:
tui-workflows-placeholder-desc = 이 워크플로가 하는 일
tui-workflows-label-steps = 단계 (JSON 배열):
tui-workflows-placeholder-steps = {"[{\"action\":\"...\"}]"}
tui-workflows-label-review = 검토 — Enter를 눌러 생성
tui-workflows-review-name =   이름:  
tui-workflows-review-desc =   설명:  
tui-workflows-hints-create-submit =   [Enter] 생성  [Esc] 뒤로
tui-workflows-hints-create-next =   [Enter] 다음  [Esc] 뒤로
tui-workflows-title-run-input = 실행: { $name }
tui-workflows-label-run-input =   입력 (JSON 또는 텍스트):
tui-workflows-placeholder-run-input = 워크플로 입력을 입력하십시오...
tui-workflows-hints-run-input =   [Enter] 실행  [Esc] 취소
tui-workflows-title-run-result = 워크플로 실행 결과
tui-workflows-running = 워크플로 실행 중...
tui-workflows-result-complete = 완료
tui-workflows-result-empty = 결과 없음.
tui-workflows-hints-run-result =   [Enter/Esc] 뒤로

# --- Triggers screen ---
tui-triggers-title-screen = 트리거
tui-triggers-header-agent = 에이전트
tui-triggers-header-pattern = 패턴
tui-triggers-header-fires = 발동
tui-triggers-header-status = 상태
tui-triggers-loading = 트리거 로드 중...
tui-triggers-empty-state = 구성된 트리거 없음. [n]으로 생성하십시오.
tui-triggers-status-active = ● 활성
tui-triggers-status-off = ○ 꺼짐
tui-triggers-create-new-option =   + 새 트리거 생성
tui-triggers-hints-list =   [↑↓] 탐색  [Enter] 생성  [d] 삭제  [r] 새로고침
tui-triggers-title-create = 새 트리거 생성
tui-triggers-create-step =   { $current } / { $total } 단계
tui-triggers-label-agent-id = 에이전트 ID:
tui-triggers-placeholder-agent-id = agent-uuid
tui-triggers-label-pattern-picker =   패턴 유형을 선택하십시오:
tui-triggers-prompt-param = { $type }의 패턴 매개변수:
tui-triggers-placeholder-pattern-param = 예: .*error.*
tui-triggers-label-prompt = 프롬프트 템플릿:
tui-triggers-placeholder-prompt = Handle this: {"{"}event{"}"}
tui-triggers-label-max-fires = 최대 발동 횟수 (0 = 무제한):
tui-triggers-placeholder-max-fires = 0
tui-triggers-review-agent =   에이전트:  
tui-triggers-review-pattern =   패턴:    
tui-triggers-review-prompt =   프롬프트:  
tui-triggers-review-max =   최대:     
tui-triggers-review-unlimited = 무제한
tui-triggers-review-confirm = Enter를 눌러 이 트리거를 생성하십시오.
tui-triggers-hints-create-submit =   [Enter] 생성  [Esc] 뒤로
tui-triggers-hints-create-select =   [↑↓] 탐색  [Enter] 선택  [Esc] 뒤로
tui-triggers-hints-create-next =   [Enter] 다음  [Esc] 뒤로

tui-triggers-type-lifecycle-name = 수명 주기
tui-triggers-type-lifecycle-desc = 에이전트 수명 주기 이벤트 (시작, 중지, 오류)
tui-triggers-type-agentspawned-name = AgentSpawned
tui-triggers-type-agentspawned-desc = 새 에이전트가 생성될 때 발동
tui-triggers-type-contentmatch-name = ContentMatch
tui-triggers-type-contentmatch-desc = 메시지 내용 일치 (정규식)
tui-triggers-type-schedule-name = Schedule
tui-triggers-type-schedule-desc = 크론 형식의 일정 트리거
tui-triggers-type-webhook-name = Webhook
tui-triggers-type-webhook-desc = HTTP 웹훅 트리거
tui-triggers-type-channelmessage-name = ChannelMessage
tui-triggers-type-channelmessage-desc = 채널에서 메시지 수신

# --- Memory screen ---
tui-memory-title-screen = 메모리
tui-memory-label-select-agent =   메모리를 탐색할 에이전트를 선택하십시오:
tui-memory-header-agent-name = 에이전트 이름
tui-memory-header-id = ID
tui-memory-loading-agents = 에이전트 로드 중...
tui-memory-empty-agents = 메모리 항목 없음. 에이전트가 여기에 데이터를 자동으로 저장합니다.
tui-memory-hints-agent-select =   ↑↓ 탐색  Enter KV 탐색  r 새로고침
tui-memory-pairs-count =   │ { $count }개 쌍
tui-memory-header-key = 키
tui-memory-header-value = 값
tui-memory-loading = 로드 중...
tui-memory-empty-kv = 키-값 쌍 없음. a를 눌러 추가하십시오.
tui-memory-confirm-delete =   이 키를 삭제하시겠습니까? [y] 예  [any] 취소
tui-memory-hints-kv-browser =   ↑↓ 탐색  a 추가  e 편집  d 삭제  Esc 뒤로  r 새로고침
tui-memory-title-add = ┼ 키-값 쌍 추가
tui-memory-title-edit = ✎ 값 편집
tui-memory-field-key = 키:
tui-memory-placeholder-key = 키를 입력하십시오...
tui-memory-field-value = 값:
tui-memory-placeholder-value = 값을 입력하십시오...
tui-memory-hints-edit =   Tab 필드 전환  Enter 저장  Esc 취소

# --- Extensions screen ---
tui-extensions-title-screen = 확장
tui-extensions-tab-browse = 1 찾아보기
tui-extensions-tab-installed = 2 설치됨
tui-extensions-tab-health = 3 상태
tui-extensions-status-ready = 준비됨
tui-extensions-status-setup = 설정
tui-extensions-status-error = 오류
tui-extensions-status-off = 꺼짐
tui-extensions-status-installed = 설치됨
tui-extensions-status-available = 사용 가능
tui-extensions-header-name = 이름
tui-extensions-header-category = 카테고리
tui-extensions-header-status = 상태
tui-extensions-header-desc = 설명
tui-extensions-header-id = ID
tui-extensions-header-server = 서버
tui-extensions-header-tools = 도구
tui-extensions-header-connected = 연결됨
tui-extensions-header-fails = 실패
tui-extensions-header-last-error = 마지막 오류
tui-extensions-loading = MCP 서버 로드 중...
tui-extensions-empty = 설치된 확장 없음. [b]로 마켓플레이스를 찾아보십시오.
tui-extensions-remove-confirm =   y를 눌러 제거를 확인하거나 다른 키로 취소하십시오
tui-extensions-hints-search =   입력하여 검색 • Esc 취소 • Enter 확인
tui-extensions-hints-browse =   j/k 탐색 • Enter 설치 • / 검색 • r 새로고침
tui-extensions-hints-installed =   j/k 탐색 • d 제거 • r 새로고침
tui-extensions-hints-health =   j/k 탐색 • r/Enter 재연결 • 자동 재연결 활성

# --- Skills screen ---
tui-skills-title-screen = 스킬
tui-skills-tab-installed = 1 설치됨
tui-skills-tab-clawhub = 2 ClawHub
tui-skills-tab-mcp = 3 MCP 서버
tui-skills-header-name = 이름
tui-skills-header-runtime = 런타임
tui-skills-header-source = 출처
tui-skills-header-desc = 설명
tui-skills-header-downloads = 다운로드
tui-skills-header-server = 서버
tui-skills-header-status = 상태
tui-skills-header-tools = 도구
tui-skills-loading = 스킬 로드 중...
tui-skills-empty = 설치된 스킬 없음. ClawHub를 찾아보고 스킬을 찾으십시오.
tui-skills-uninstall-confirm =   이 스킬을 제거하시겠습니까? [y] 예  [any] 취소
tui-skills-hints-installed =   [↑↓] 탐색  [u] 제거  [r] 새로고침
tui-skills-sort =   정렬: { $sort }
tui-skills-sort-trending = 인기 급상승
tui-skills-sort-popular = 인기
tui-skills-sort-recent = 최신
tui-skills-searching = ClawHub 검색 중...
tui-skills-empty-clawhub = 결과 없음. [/]로 검색하거나 [s]로 정렬을 변경하십시오.
tui-skills-hints-clawhub =   [↑↓] 탐색  [i] 설치  [/] 검색  [s] 정렬  [r] 새로고침
tui-skills-loading-mcp = MCP 서버 로드 중...
tui-skills-empty-mcp = 구성된 MCP 서버 없음. config.toml에서 서버를 추가하십시오.
tui-skills-hints-mcp =   [↑↓] 탐색  [r] 새로고침
tui-skills-mcp-status-connected = 연결됨
tui-skills-mcp-status-disconnected = 연결 끊김
tui-skills-mcp-tools-count = { $count }개 도구

# --- Setup Wizard screen ---
tui-wizard-title = 설정
tui-wizard-step-1 = 1/3 단계
tui-wizard-step-2 = 2/3 단계
tui-wizard-step-3 = 3/3 단계
tui-wizard-step-saving = 저장 중...
tui-wizard-step-complete = 완료
tui-wizard-prompt-provider = LLM 공급자를 선택하십시오:
tui-wizard-hint-cli-detected = CLI 감지됨
tui-wizard-hint-no-key-needed = API 키 필요 없음
tui-wizard-hint-local-no-key = 로컬, 키 필요 없음
tui-wizard-hint-env-detected = { $env } 감지됨
tui-wizard-hint-env-required = { $env } 필요
tui-wizard-hints-provider =     [↑↓] 탐색  [Enter] 선택  [Esc] 취소
tui-wizard-prompt-api-key = { $provider } API 키를 입력하십시오:
tui-wizard-hint-env-alternative = 또는 { $env } 환경 변수를 설정하십시오
tui-wizard-hints-confirm-back =     [Enter] 확인  [Esc] 뒤로
tui-wizard-prompt-model-name = 모델 이름:
tui-wizard-hint-model-default = 기본값: { $model }
tui-wizard-status-no-provider = 선택된 공급자 없음
tui-wizard-status-no-home = 홈 디렉터리를 확인할 수 없음
tui-wizard-status-saved = 구성 저장됨 — { $provider } / { $model }
tui-wizard-status-save-fail = 구성 저장에 실패: { $error }
tui-wizard-status-continuing = 계속하는 중...




