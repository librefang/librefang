# --- API error messages (Korean) ---

# Agent errors
api-error-agent-not-found = 에이전트를 찾을 수 없습니다
api-error-agent-spawn-failed = 에이전트 생성에 실패했습니다
api-error-agent-invalid-id = 유효하지 않은 에이전트 ID입니다
api-error-agent-already-exists = 에이전트가 이미 존재합니다
api-error-agent-no-workspace = 에이전트에 작업 공간이 없습니다
api-error-agent-not-found-or-terminated = 에이전트를 찾을 수 없거나 이미 종료되었습니다
api-error-agent-vanished = 업데이트 중 에이전트가 사라졌습니다
api-error-agent-no-agents-available = 사용 가능한 에이전트가 없습니다
api-error-agent-no-target = 대상 에이전트를 찾을 수 없습니다. agent_id를 지정하거나 먼저 에이전트를 시작하십시오.
api-error-agent-source-not-found = 소스 에이전트를 찾을 수 없습니다
api-error-agent-target-not-found = 대상 에이전트를 찾을 수 없습니다
api-error-agent-execution-failed = 에이전트 실행에 실패했습니다: { $error }
api-error-agent-clone-spawn-failed = 클론을 생성하는 데 실패했습니다: { $error }
api-error-agent-error = 에이전트 오류: { $error }
api-error-agent-not-found-with-id = 에이전트를 찾을 수 없습니다: { $id }
api-error-agent-invalid-sort = 유효하지 않은 정렬 필드 '{ $field }'입니다. 유효한 필드: { $valid }

# Message errors
api-error-message-too-large = 메시지가 너무 큽니다 (최대 64KB)
api-error-message-delivery-failed = 메시지 전송에 실패했습니다: { $reason }
api-error-message-required = 메시지가 필요합니다
api-error-message-missing-field = 'message' 필드가 누락되었습니다
api-error-message-streaming-failed = 스트리밍 메시지를 전송하는 데 실패했습니다

# Template errors
api-error-template-invalid-name = 유효하지 않은 템플릿 이름입니다
api-error-template-not-found = 템플릿 '{ $name }'을(를) 찾을 수 없습니다
api-error-template-parse-failed = 템플릿 분석에 실패했습니다: { $error }
api-error-template-required = 'manifest_toml' 또는 'template' 중 하나가 필요합니다
api-error-template-invalid-manifest = 유효하지 않은 템플릿 매니페스트입니다
api-error-template-read-failed = 템플릿을 읽는 데 실패했습니다

# Manifest errors
api-error-manifest-too-large = 매니페스트가 너무 큽니다 (최대 1MB)
api-error-manifest-invalid-format = 유효하지 않은 매니페스트 형식입니다
api-error-manifest-signature-mismatch = 서명된 매니페스트 내용이 manifest_toml과 일치하지 않습니다
api-error-manifest-signature-failed = 매니페스트 서명 검증에 실패했습니다
api-error-manifest-invalid = 유효하지 않은 매니페스트입니다: { $error }

# Auth errors
api-error-auth-invalid-key = 유효하지 않은 API 키입니다
api-error-auth-missing-header = Authorization: Bearer <api_key> 헤더가 누락되었습니다
api-error-auth-missing = 이 공급자에 대한 API 키가 구성되지 않았습니다

# Session errors
api-error-session-load-failed = 세션 로드에 실패했습니다
api-error-session-not-found = 세션을 찾을 수 없습니다
api-error-session-invalid-id = 유효하지 않은 세션 ID입니다
api-error-context-report-failed = 컨텍스트 보고에 실패했습니다
api-error-session-no-label = 해당 레이블을 가진 세션을 찾을 수 없습니다
api-error-session-cleanup-expired-failed = 만료된 세션을 정리하는 데 실패했습니다: { $error }
api-error-session-cleanup-excess-failed = 초과된 세션을 정리하는 데 실패했습니다: { $error }

# Workflow errors
api-error-workflow-missing-steps = 'steps' 배열이 누락되었습니다
api-error-workflow-step-needs-agent = 단계 '{ $step }'에 'agent_id' 또는 'agent_name'이 필요합니다
api-error-workflow-invalid-id = 유효하지 않은 워크플로 ID입니다
api-error-workflow-execution-failed = 워크플로 실행에 실패했습니다
api-error-workflow-not-found = 워크플로를 찾을 수 없습니다

# Trigger errors
api-error-trigger-missing-agent-id = 'agent_id'가 누락되었습니다
api-error-trigger-invalid-agent-id = 유효하지 않은 agent_id입니다
api-error-trigger-invalid-pattern = 유효하지 않은 트리거 패턴입니다
api-error-trigger-missing-pattern = 'pattern'이 누락되었습니다
api-error-trigger-registration-failed = 트리거 등록에 실패했습니다 (에이전트를 찾을 수 없습니까?)
api-error-trigger-invalid-id = 유효하지 않은 트리거 ID입니다
api-error-trigger-not-found = 트리거를 찾을 수 없습니다

# Budget errors
api-error-budget-invalid-amount = 유효하지 않은 예산 금액입니다
api-error-budget-update-failed = 예산 업데이트에 실패했습니다
api-error-budget-provide-at-least-one = 다음 중 하나 이상을 제공하십시오: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour

# Config errors
api-error-config-parse-failed = 설정을 분석하는 데 실패했습니다: { $error }
api-error-config-write-failed = 설정을 저장하는 데 실패했습니다: { $error }
api-error-config-save-failed = 설정을 저장하는 데 실패했습니다: { $error }
api-error-config-remove-failed = 설정을 삭제하는 데 실패했습니다: { $error }
api-error-config-missing-toml = toml_content 필드가 누락되었습니다

# Profile errors
api-error-profile-not-found = 프로필 '{ $name }'을(를) 찾을 수 없습니다

# Cron errors
api-error-cron-invalid-id = 유효하지 않은 크론 작업 ID입니다
api-error-cron-not-found = 크론 작업을 찾을 수 없습니다
api-error-cron-create-failed = 크론 작업을 생성하는 데 실패했습니다: { $error }
api-error-cron-invalid-expression = 유효하지 않은 크론 표현식입니다
api-error-cron-invalid-expression-detail = 유효하지 않은 크론 표현식입니다: 5개의 필드가 필요합니다 (분 시 일 월 요일)
api-error-cron-missing-field = 'cron' 필드가 누락되었습니다

# Goal errors
api-error-goal-not-found = 목표를 찾을 수 없습니다
api-error-goal-not-found-with-id = 목표 '{ $id }'을(를) 찾을 수 없습니다
api-error-goal-missing-title = 'title' 필드가 누락되었거나 비어 있습니다
api-error-goal-title-too-long = 제목이 너무 깁니다 (최대 256자)
api-error-goal-description-too-long = 설명이 너무 깁니다 (최대 4096자)
api-error-goal-invalid-status = 유효하지 않은 상태입니다. 다음 중 하나여야 합니다: pending, in_progress, completed, cancelled
api-error-goal-progress-range = 진행률은 0-100 범위여야 합니다
api-error-goal-parent-not-found = 상위 목표 '{ $id }'을(를) 찾을 수 없습니다
api-error-goal-self-parent = 목표는 자기 자신을 상위 목표로 가질 수 없습니다
api-error-goal-circular-parent = 순환 상위 목표 참조가 감지되었습니다
api-error-goal-save-failed = 목표를 저장하는 데 실패했습니다: { $error }
api-error-goal-update-failed = 목표를 업데이트하는 데 실패했습니다: { $error }
api-error-goal-delete-failed = 목표를 삭제하는 데 실패했습니다: { $error }
api-error-goal-load-failed = 목표를 로드하는 데 실패했습니다: { $error }
api-error-goal-title-empty = 제목은 비워 둘 수 없습니다
api-error-goal-status-invalid = 유효하지 않은 상태입니다

# Memory errors
api-error-memory-not-enabled = 능동적 메모리가 활성화되지 않았습니다
api-error-memory-not-found = 메모리를 찾을 수 없습니다
api-error-memory-operation-failed = 메모리 작업에 실패했습니다
api-error-memory-export-failed = 메모리를 내보내는 데 실패했습니다
api-error-memory-import-failed = 초기화 중 메모리를 가져오는 데 실패했습니다
api-error-memory-key-not-found = 키를 찾을 수 없습니다
api-error-memory-missing-kv = 요청 본문에 'kv' 객체가 누락되었거나 유효하지 않습니다
api-error-memory-serialization-error = 직렬화 오류
api-error-memory-missing-ids = 'ids' 배열이 누락되었습니다

# Network / A2A errors
api-error-network-not-enabled = 피어 네트워크가 활성화되지 않았습니다
api-error-network-peer-not-found = 피어를 찾을 수 없습니다
api-error-network-a2a-not-found = A2A 에이전트 '{ $url }'을(를) 찾을 수 없습니다
api-error-network-connection-failed = 연결에 실패했습니다: { $error }
api-error-network-auth-failed = 인증에 실패했습니다 (HTTP { $status })
api-error-network-task-post-failed = 작업을 게시하는 데 실패했습니다: { $error }
api-error-network-missing-url = 'url' 쿼리 매개변수가 누락되었습니다

# Plugin errors
api-error-plugin-missing-name = 'name'이(가) 누락되었습니다
api-error-plugin-missing-name-registry = 레지스트리 설치에 필요한 'name'이(가) 누락되었습니다
api-error-plugin-missing-path = 로컬 설치에 필요한 'path'가 누락되었습니다
api-error-plugin-missing-url = git 설치에 필요한 'url'이(가) 누락되었습니다
api-error-plugin-invalid-source = 유효하지 않은 소스입니다. 'registry', 'local', 'git' 중 하나를 사용하십시오

# Channel errors
api-error-channel-unknown = 알 수 없는 채널입니다
api-error-channel-missing-agent-id = 필수 필드가 누락되었습니다: agent_id
api-error-channel-invalid-from = 유효하지 않은 from_agent_id입니다
api-error-channel-invalid-to = 유효하지 않은 to_agent_id입니다

# Provider errors
api-error-provider-missing-alias = 필수 필드가 누락되었습니다: alias
api-error-provider-missing-model-id = 필수 필드가 누락되었습니다: model_id
api-error-provider-missing-id = 필수 필드가 누락되었습니다: id
api-error-provider-missing-key = 'key' 필드가 누락되었거나 비어 있습니다
api-error-provider-alias-exists = 별칭 '{ $alias }'이(가) 이미 존재합니다
api-error-provider-alias-not-found = 별칭 '{ $alias }'을(를) 찾을 수 없습니다
api-error-provider-model-not-found = 모델 '{ $id }'을(를) 찾을 수 없습니다
api-error-provider-not-found = 공급자 '{ $name }'을(를) 찾을 수 없습니다
api-error-provider-model-exists = 모델 '{ $id }'이(가) 공급자 '{ $provider }'에 이미 존재합니다
api-error-provider-custom-model-not-found = 사용자 지정 모델 '{ $id }'을(를) 찾을 수 없습니다
api-error-provider-no-key-required = 이 공급자는 API 키가 필요하지 않습니다
api-error-provider-key-not-configured = 공급자 API 키가 구성되지 않았습니다
api-error-provider-secrets-write-failed = secrets.env를 저장하는 데 실패했습니다: { $error }
api-error-provider-secrets-update-failed = secrets.env를 업데이트하는 데 실패했습니다: { $error }
api-error-provider-invalid-url = 유효하지 않은 URL 형식입니다
api-error-provider-missing-url = 'url'이(가) 누락되었거나 비어 있습니다
api-error-provider-missing-base-url = 'base_url' 필드가 누락되었거나 비어 있습니다
api-error-provider-unknown = 알 수 없는 공급자 '{ $name }'입니다
api-error-provider-base-url-invalid = base_url은 http:// 또는 https://로 시작해야 합니다
api-error-provider-missing-model = 'model' 필드가 누락되었습니다
api-error-provider-token-save-failed = 토큰을 저장하는 데 실패했습니다: { $error }
api-error-provider-unknown-poll = 알 수 없는 poll_id입니다
api-error-provider-secret-write-failed = 시크릿을 저장하는 데 실패했습니다: { $error }

# Skill errors
api-error-skill-missing-name = 'name' 필드가 누락되었거나 비어 있습니다
api-error-skill-invalid-name = 스킬 이름에는 영숫자, 하이픈, 밑줄만 사용할 수 있습니다
api-error-skill-not-found-source = 이 스킬의 소스 코드를 찾을 수 없습니다
api-error-skill-only-prompt = 웹 UI에서는 프롬프트 전용 스킬만 생성할 수 있습니다
api-error-skill-name-too-long = 이름이 최대 길이(256자)를 초과합니다
api-error-skill-description-too-long = 설명이 최대 길이({ $max }자)를 초과합니다
api-error-skill-dir-create-failed = 스킬 디렉터리를 생성하는 데 실패했습니다: { $error }
api-error-skill-toml-write-failed = skill.toml을 저장하는 데 실패했습니다: { $error }
api-error-skill-install-failed = 설치에 실패했습니다: { $error }

# Hand errors
api-error-hand-not-found = 핸드를 찾을 수 없습니다: { $id }
api-error-hand-definition-not-found = 핸드 정의를 찾을 수 없습니다
api-error-hand-instance-not-found = 인스턴스를 찾을 수 없습니다

# MCP errors
api-error-mcp-missing-name = 'name' 필드가 누락되었습니다
api-error-mcp-missing-transport = 'transport' 필드가 누락되었습니다
api-error-mcp-invalid-config = 유효하지 않은 MCP 서버 설정입니다: { $error }
api-error-mcp-not-found = MCP 서버 '{ $name }'을(를) 찾을 수 없습니다
api-error-mcp-write-failed = 설정을 저장하는 데 실패했습니다: { $error }

# Integration/Extension errors
api-error-integration-not-found = 통합 '{ $id }'을(를) 찾을 수 없습니다
api-error-integration-missing-id = 'id' 필드가 누락되었습니다
api-error-extension-not-found = 확장 '{ $id }'을(를) 찾을 수 없습니다

# System errors
api-error-system-cli-not-found = PATH에서 CLI를 찾을 수 없습니다

# KV / Structured memory errors
api-error-kv-missing-fields = 'fields' 객체가 누락되었습니다
api-error-kv-missing-value = 'value' 필드가 누락되었습니다
api-error-kv-array-empty = 배열은 비워 둘 수 없습니다
api-error-kv-missing-path = 'path' 필드가 누락되었습니다

# Approval errors
api-error-approval-invalid-id = 유효하지 않은 승인 ID입니다
api-error-approval-not-found = 승인을 찾을 수 없습니다

# Webhook errors
api-error-webhook-not-enabled = 웹훅 트리거가 활성화되지 않았습니다
api-error-webhook-invalid-id = 유효하지 않은 웹훅 ID입니다
api-error-webhook-not-found = 웹훅을 찾을 수 없습니다
api-error-webhook-missing-url = 'url' 필드가 누락되었습니다
api-error-webhook-missing-events = 'events' 배열이 누락되었습니다
api-error-webhook-invalid-events = 이벤트 유형은 문자열이어야 합니다
api-error-webhook-event-types-required = 최소 하나의 이벤트 유형이 필요합니다
api-error-webhook-url-unreachable = 웹훅 URL에 연결할 수 없습니다: { $error }
api-error-webhook-event-publish-failed = 이벤트를 게시하는 데 실패했습니다: { $error }
api-error-webhook-invalid-url = 유효하지 않은 웹훅 URL 형식입니다
api-error-webhook-agent-exec-failed = 웹훅 에이전트 실행에 실패했습니다: { $error }
api-error-webhook-reach-failed = 웹훅 URL에 연결하는 데 실패했습니다: { $error }
api-error-webhook-unknown-event = 알 수 없는 이벤트 유형 '{ $event }'입니다. 유효한 유형: { $valid }

# Backup errors
api-error-backup-not-found = 백업을 찾을 수 없습니다
api-error-backup-file-not-found = 백업 파일을 찾을 수 없습니다
api-error-backup-invalid-filename = 유효하지 않은 백업 파일 이름입니다
api-error-backup-invalid-filename-zip = 유효하지 않은 백업 파일 이름입니다 — .zip 파일이어야 합니다
api-error-backup-missing-manifest = 백업 아카이브에 manifest.json이 누락되었습니다 — 유효한 LibreFang 백업이 아닙니다
api-error-backup-dir-create-failed = 백업 디렉터리를 생성하는 데 실패했습니다: { $error }
api-error-backup-file-create-failed = 백업 파일을 생성하는 데 실패했습니다: { $error }
api-error-backup-finalize-failed = 백업을 완료하는 데 실패했습니다: { $error }
api-error-backup-open-failed = 백업을 여는 데 실패했습니다: { $error }
api-error-backup-invalid-archive = 유효하지 않은 백업 아카이브입니다: { $error }
api-error-backup-delete-failed = 백업을 삭제하는 데 실패했습니다: { $error }

# Schedule errors
api-error-schedule-not-found = 일정을 찾을 수 없습니다
api-error-schedule-missing-cron = 'cron' 필드가 누락되었습니다
api-error-schedule-missing-enabled = 'enabled' 필드가 누락되었습니다
api-error-schedule-invalid-cron = 유효하지 않은 크론 표현식입니다
api-error-schedule-invalid-cron-detail = 유효하지 않은 크론 표현식입니다: 5개의 필드가 필요합니다 (분 시 일 월 요일)
api-error-schedule-save-failed = 일정을 저장하는 데 실패했습니다: { $error }
api-error-schedule-update-failed = 일정을 업데이트하는 데 실패했습니다: { $error }
api-error-schedule-delete-failed = 일정을 삭제하는 데 실패했습니다: { $error }
api-error-schedule-load-failed = 일정을 로드하는 데 실패했습니다: { $error }

# Job errors
api-error-job-invalid-id = 유효하지 않은 작업 ID입니다
api-error-job-not-found = 작업을 찾을 수 없습니다
api-error-job-not-retryable = 작업을 찾을 수 없거나 재시도 가능한 상태가 아닙니다(완료 또는 실패 상태여야 합니다)
api-error-job-disappeared-cancel = 취소 후 작업이 사라졌습니다
api-error-job-disappeared-complete = 완료 후 작업이 사라졌습니다

# Task errors
api-error-task-not-found = 작업을 찾을 수 없습니다
api-error-task-disappeared = 작업이 사라졌습니다

# Pairing errors
api-error-pairing-not-enabled = 페어링이 활성화되지 않았습니다
api-error-pairing-invalid-token = 유효하지 않거나 누락된 토큰입니다

# Binding errors
api-error-binding-out-of-range = 바인딩 인덱스가 범위를 벗어났습니다

# Command errors
api-error-command-not-found = 명령 '{ $name }'을(를) 찾을 수 없습니다

# File/Upload errors
api-error-file-not-found = 파일을 찾을 수 없습니다
api-error-file-not-in-whitelist = 파일이 화이트리스트에 없습니다
api-error-file-too-large = 파일이 너무 큽니다 (최대 { $max })
api-error-file-content-too-large = 파일 내용이 너무 큽니다 (최대 32KB)
api-error-file-empty-body = 파일 본문이 비어 있습니다
api-error-file-save-failed = 파일을 저장하는 데 실패했습니다
api-error-file-missing-filename = 'filename' 필드가 누락되었습니다
api-error-file-missing-path = 'path' 필드가 누락되었습니다
api-error-file-path-too-deep = 경로가 너무 깊습니다 (최대 3단계)
api-error-file-path-traversal = 경로 탐색이 거부되었습니다
api-error-file-unsupported-type = 지원되지 않는 콘텐츠 유형입니다. 허용: image/*, text/*, audio/*, application/pdf
api-error-file-upload-dir-failed = 업로드 디렉터리를 생성하는 데 실패했습니다
api-error-file-dir-not-found = 디렉터리를 찾을 수 없습니다
api-error-file-workspace-error = 작업 공간 경로 오류

# Tool errors
api-error-tool-provide-allowlist = 'tool_allowlist' 및/또는 'tool_blocklist'를 제공하십시오
api-error-tool-not-found = 도구를 찾을 수 없습니다: { $name }
api-error-tool-invoke-disabled = 직접 도구 호출이 비활성화되어 있습니다. '[tool_invoke] enabled = true'를 활성화하고 도구를 'allowlist'에 추가하십시오.
api-error-tool-invoke-denied = 도구 '{ $name }'이(가) '[tool_invoke] allowlist'에 없습니다
api-error-tool-requires-agent = 도구 '{ $name }'은(는) 사람의 승인이 필요하며 에이전트 컨텍스트 없이는 호출할 수 없습니다. 대신 에이전트를 통해 호출하십시오

# Validation errors
api-error-validation-content-empty = 콘텐츠는 비워 둘 수 없습니다
api-error-validation-name-empty = new_name은(는) 비워 둘 수 없습니다
api-error-validation-title-required = 제목이 필요합니다
api-error-validation-avatar-url-invalid = 아바타 URL은 http/https 또는 data URI여야 합니다
api-error-validation-color-invalid = 색상은 '#'으로 시작하는 hex 코드여야 합니다

# General errors
api-error-not-found = 리소스를 찾을 수 없습니다
api-error-internal = 내부 서버 오류
api-error-bad-request = 잘못된 요청: { $reason }
api-error-rate-limited = 요청 제한을 초과했습니다. 나중에 다시 시도하십시오.

# Generic catch-all — interpolates the underlying error string verbatim.
# Used by 41+ HTTP 500 handlers as a stopgap until each route is moved to a
# typed MemoryRouteError-style helper. Without this key, every `t_args("api-error-generic", …)`
# call returns the literal key as the response body and `$error` interpolation never runs.
api-error-generic = 오류: { $error }
