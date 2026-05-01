import { ApiError } from "./lib/http/errors";

export interface HealthCheck {
  name: string;
  status: string;
}

export interface HealthResponse {
  status?: string;
  checks?: HealthCheck[];
}

export interface StatusResponse {
  version?: string;
  agent_count?: number;
  active_agent_count?: number;
  memory_used_mb?: number;
  uptime_seconds?: number;
  default_provider?: string;
  default_model?: string;
  api_listen?: string;
  home_dir?: string;
  log_level?: string;
  /** Machine hostname. Only populated on authenticated endpoints
   *  (`/api/status`, `/api/dashboard/snapshot`) — `/api/version` is public
   *  and deliberately omits it. */
  hostname?: string;
  network_enabled?: boolean;
  terminal_enabled?: boolean;
  session_count?: number;
  config_exists?: boolean;
}

export interface VersionResponse {
  name?: string;
  version?: string;
  build_date?: string;
  git_sha?: string;
  rust_version?: string;
  platform?: string;
  arch?: string;
  hostname?: string;
}

export interface ProviderItem {
  id: string;
  display_name?: string;
  auth_status?: string;
  reachable?: boolean;
  model_count?: number;
  latency_ms?: number;
  api_key_env?: string;
  base_url?: string;
  proxy_url?: string;
  key_required?: boolean;
  health?: string;
  media_capabilities?: string[];
  is_custom?: boolean;
  error_message?: string;
  last_tested?: string;
}

export interface MediaProvider {
  name: string;
  configured: boolean;
  capabilities: string[];
}

export interface MediaImageResult {
  images: { data_base64: string; url?: string }[];
  model: string;
  provider: string;
  revised_prompt?: string;
}

export interface MediaTtsResult {
  format: string;
  provider: string;
  model: string;
  duration_ms?: number;
}

export interface MediaVideoSubmitResult {
  task_id: string;
  provider: string;
}

export interface MediaVideoResult {
  file_url: string;
  width?: number;
  height?: number;
  duration_secs?: number;
  provider: string;
  model: string;
}

export interface MediaVideoStatus {
  status: string;
  task_id?: string;
  result?: MediaVideoResult;
  error?: string;
}

export interface MediaMusicResult {
  url: string;
  format: string;
  provider: string;
  model: string;
  duration_ms?: number;
  sample_rate?: number;
}

export interface ChannelField {
  key: string;
  label?: string;
  type?: string;
  required?: boolean;
  advanced?: boolean;
  has_value?: boolean;
  env_var?: string | null;
  placeholder?: string | null;
  value?: string;
  options?: string[];
  show_when?: string;
  readonly?: boolean;
}

export interface ChannelItem {
  name: string;
  display_name?: string;
  configured?: boolean;
  has_token?: boolean;
  category?: string;
  description?: string;
  icon?: string;
  difficulty?: string;
  setup_time?: string;
  quick_setup?: string;
  setup_type?: string;
  setup_steps?: string[];
  fields?: ChannelField[];
  /** Webhook endpoint path on the shared server (e.g. "/channels/feishu/webhook"). */
  webhook_endpoint?: string;
}

export interface SkillItem {
  name: string;
  version?: string;
  description?: string;
  runtime?: string;
  enabled?: boolean;
  author?: string;
  tools_count?: number;
  tags?: string[];
  source?: {
    type?: string;
    slug?: string;
    version?: string;
  };
}

export interface SkillsResponse {
  skills?: SkillItem[];
  total?: number;
  categories?: string[];
}

// Skill evolution types
export interface SkillVersionEntry {
  version: string;
  timestamp: string;
  changelog: string;
  content_hash: string;
  author?: string | null;
}

export interface SkillEvolutionMeta {
  versions: SkillVersionEntry[];
  use_count: number;
  /** Total version entries written, incl. initial creation. */
  evolution_count: number;
  /** Mutations after creation (update/patch/rollback). 0 on fresh skill. */
  mutation_count: number;
}

export interface SkillToolInfo {
  name: string;
  description: string;
}

export interface SkillDetail {
  name: string;
  version: string;
  description: string;
  author: string;
  license: string;
  tags: string[];
  runtime: string;
  tools: SkillToolInfo[];
  has_prompt_context: boolean;
  prompt_context_length: number;
  prompt_context?: string | null;
  /** Backend-supplied skill source descriptor. Treated as opaque on the
   *  dashboard — no UI introspects it today, so leave the shape unconstrained
   *  rather than freezing a partial picture into the type. */
  source: unknown;
  enabled: boolean;
  path: string;
  linked_files: Record<string, string[]>;
  evolution: SkillEvolutionMeta;
}

export interface EvolutionResult {
  success: boolean;
  message: string;
  skill_name: string;
  version?: string;
  /** Set only on patch ops: which fuzzy strategy matched. */
  match_strategy?: "Exact" | "WhitespaceStripped" | "LineTrimmed" | "WhitespaceNormalized" | "IndentFlexible" | "BlockAnchor";
  /** Set only on patch ops: replaced occurrence count. */
  match_count?: number;
  /** Post-op version-history size (includes initial creation). */
  evolution_count?: number;
  /** Post-op mutation counter (post-create edits only — 0 on fresh create). */
  mutation_count?: number;
  /** Post-op usage counter. */
  use_count?: number;
}

export interface ProvidersResponse {
  providers?: ProviderItem[];
  total?: number;
}

export interface ChannelsResponse {
  channels?: ChannelItem[];
  total?: number;
  configured_count?: number;
}

export interface DashboardSnapshot {
  health: HealthResponse;
  status: StatusResponse;
  providers: ProviderItem[];
  channels: ChannelItem[];
  agents: AgentItem[];
  skillCount: number;
  workflowCount: number;
  webSearchAvailable: boolean;
}

export interface AgentIdentity {
  emoji?: string;
  avatar_url?: string;
  color?: string;
}

export interface AgentItem {
  id: string;
  name: string;
  state?: string;
  mode?: string;
  created_at?: string;
  last_active?: string;
  model_provider?: string;
  model_name?: string;
  model_tier?: string;
  auth_status?: string;
  supports_thinking?: boolean;
  ready?: boolean;
  profile?: string;
  /** Human-readable schedule summary: "manual" for reactive agents,
   *  the cron expression for periodic agents, "proactive", or
   *  "continuous · Ns" for continuous agents. */
  schedule?: string;
  /** Sessions whose `created_at` is within the last 24 hours. Computed
   *  in a single grouped SQL pass on the list endpoint so row UIs can
   *  render KPI without a global /api/sessions aggregation. */
  sessions_24h?: number;
  /** Sum of `usage_events.cost_usd` for the agent in the last 24 hours. */
  cost_24h?: number;
  identity?: AgentIdentity;
  is_hand?: boolean;
  web_search_augmentation?: "off" | "auto" | "always";
  /** UUID of the parent agent that spawned this one, if any. */
  parent_agent_id?: string;
  /** UUIDs of child agents spawned by this agent. */
  children?: string[];
  /** Active session UUID. */
  session_id?: string;
  /** Categorisation tags. */
  tags?: string[];
}

export interface PaginatedResponse<T> {
  items?: T[];
  total?: number;
  offset?: number;
  limit?: number | null;
}

export interface AgentTool {
  name?: string;
  input?: unknown;
  result?: string;
  is_error?: boolean;
  running?: boolean;
  expanded?: boolean;
}

export interface AgentSessionImage {
  file_id: string;
  filename?: string;
  /** Optional — backend currently only emits image attachments here, so
   *  the renderer treats a missing value as image. Threading the field
   *  through keeps the chat transcript correct if the server starts
   *  serializing non-image attachments (PDF/text) into history. */
  content_type?: string;
}

/** Reference passed back to the agent's `/message` endpoint or WS frame
 *  after a successful upload. Mirrors `crate::types::AttachmentRef`. */
export interface AttachmentRef {
  file_id: string;
  filename?: string;
  content_type?: string;
}

export interface AgentFileUploadResult {
  file_id: string;
  filename: string;
  content_type: string;
  size: number;
  /** Whisper transcription, populated only for audio uploads. */
  transcription?: string;
}

export interface AgentSessionMessage {
  role?: string;
  content?: unknown;
  tools?: AgentTool[];
  images?: AgentSessionImage[];
  /** RFC 3339 timestamp from the server; may be absent for messages
   * persisted before the field was introduced. */
  timestamp?: string;
}

export interface AgentSessionResponse {
  session_id?: string;
  agent_id?: string;
  message_count?: number;
  context_window_tokens?: number;
  label?: string;
  messages?: AgentSessionMessage[];
}

export interface AgentMessageResponse {
  response?: string;
  input_tokens?: number;
  output_tokens?: number;
  iterations?: number;
  cost_usd?: number;
  silent?: boolean;
  memories_saved?: string[];
  memories_used?: string[];
  thinking?: string;
}

export interface SendAgentMessageOptions {
  /** Force deep-thinking on/off for this call. Omitted = manifest default. */
  thinking?: boolean;
  /** Whether to receive the model's reasoning trace. Defaults to true. */
  show_thinking?: boolean;
  /**
   * Optional explicit session id (issue #2959). When provided, this send
   * targets the given session regardless of the agent's canonical session.
   * Used by the chat UI when a specific session is selected in the URL so
   * two browser tabs on the same agent don't race each other.
   */
  session_id?: string | null;
  /** File attachments uploaded via `/api/agents/{id}/upload`. */
  attachments?: AttachmentRef[];
}

export interface ApiActionResponse {
  status?: string;
  message?: string;
  error?: string;
  [key: string]: unknown;
}

export interface WorkflowStep {
  name: string;
  agent_id?: string;
  agent_name?: string;
  prompt_template: string;
  timeout_secs?: number;
  inherit_context?: boolean;
  depends_on?: string[];
}

export interface WorkflowItem {
  id: string;
  name: string;
  description?: string;
  steps?: number | WorkflowStep[];
  created_at?: string;
  layout?: unknown;
}

export interface WorkflowRunItem {
  id?: string;
  workflow_name?: string;
  state?: unknown;
  steps_completed?: number;
  started_at?: string;
  completed_at?: string | null;
}

/**
 * Multi-destination cron output fan-out target.
 *
 * Mirrors the Rust enum `librefang_types::scheduler::CronDeliveryTarget`,
 * which is `#[serde(tag = "type", rename_all = "snake_case")]`. Each variant
 * is an object with a `type` discriminator plus variant-specific fields.
 *
 * Empty/optional fields (`auth_header`, `subject_template`) MUST be omitted
 * from the payload rather than sent as empty strings — the Rust side uses
 * `Option<String>` and treating `""` as `Some("")` would leak through.
 */
export type CronDeliveryTarget =
  | {
      type: "channel";
      /** Adapter name, e.g. "telegram", "slack", "discord". */
      channel_type: string;
      /** Platform-specific recipient (chat ID, user ID, channel ID). */
      recipient: string;
      /**
       * Optional thread/topic id (Slack `thread_ts`, Telegram forum-topic
       * id). Omit unless the adapter supports threading. Empty strings are
       * stripped at submit time so the wire shape matches the Rust
       * `Option<String>` exactly.
       */
      thread_id?: string;
      /**
       * Optional adapter-key suffix used to disambiguate multiple
       * configured accounts of the same channel (e.g. two Slack workspaces
       * keyed `slack:workspace-a` vs `slack:workspace-b`). Omit when only
       * one account of `channel_type` is configured.
       */
      account_id?: string;
    }
  | {
      type: "webhook";
      /** Destination URL. Must start with http:// or https://. */
      url: string;
      /** Optional Authorization header value (sent verbatim). */
      auth_header?: string;
    }
  | {
      type: "local_file";
      /** Absolute or relative path on the daemon host. */
      path: string;
      /** If true, append to the file; if false, overwrite. */
      append?: boolean;
    }
  | {
      type: "email";
      /** Recipient email address. */
      to: string;
      /** Optional subject template. `{job}` is replaced with the job name. */
      subject_template?: string;
    };

/** Discriminator string for `CronDeliveryTarget` — useful for switch arms. */
export type CronDeliveryTargetType = CronDeliveryTarget["type"];

export interface ScheduleItem {
  id: string;
  name?: string;
  cron?: string;
  tz?: string | null;
  description?: string;
  message?: string;
  enabled?: boolean;
  created_at?: string;
  last_run?: string | null;
  next_run?: string | null;
  agent_id?: string;
  workflow_id?: string;
  /**
   * Optional fan-out destinations. Empty/missing means single-target
   * delivery via the legacy `delivery` field. Backend sends an array
   * (possibly empty) on round-trip.
   */
  delivery_targets?: CronDeliveryTarget[];
}

export interface TriggerItem {
  id: string;
  agent_id?: string;
  pattern?: unknown;
  prompt_template?: string;
  enabled?: boolean;
  fire_count?: number;
  max_fires?: number;
  created_at?: string;
  target_agent_id?: string | null;
  cooldown_secs?: number | null;
  session_mode?: string | null;
}

export interface TriggerPatch {
  pattern?: unknown;
  prompt_template?: string;
  enabled?: boolean;
  max_fires?: number;
  cooldown_secs?: number | null;
  session_mode?: string | null;
  target_agent_id?: string | null;
}

export interface CreateTriggerPayload {
  agent_id: string;
  pattern: unknown;
  prompt_template: string;
  max_fires?: number;
  target_agent_id?: string;
  cooldown_secs?: number;
  session_mode?: string;
}

export interface CronJobItem {
  id?: string;
  enabled?: boolean;
  name?: string;
  schedule?: string;
  [key: string]: unknown;
}

export interface QueueLaneStatus {
  lane?: string;
  active?: number;
  capacity?: number;
}

export interface QueueConcurrencyConfig {
  main_lane?: number;
  cron_lane?: number;
  subagent_lane?: number;
  trigger_lane?: number;
  default_per_agent?: number;
}

export interface QueueStatusResponse {
  lanes?: QueueLaneStatus[];
  config?: {
    max_depth_per_agent?: number;
    max_depth_global?: number;
    task_ttl_secs?: number;
    concurrency?: QueueConcurrencyConfig;
  };
}

export interface AuditEntry {
  seq?: number;
  timestamp?: string;
  agent_id?: string;
  action?: string;
  detail?: string;
  outcome?: string;
  hash?: string;
}

export interface AuditRecentResponse {
  entries?: AuditEntry[];
  total?: number;
  tip_hash?: string;
}

export interface AuditVerifyResponse {
  valid?: boolean;
  entries?: number;
  tip_hash?: string;
  warning?: string;
  error?: string;
}

export interface ApprovalItem {
  id: string;
  agent_id?: string;
  agent_name?: string;
  tool_name?: string;
  description?: string;
  action_summary?: string;
  action?: string;
  risk_level?: string;
  requested_at?: string;
  created_at?: string;
  timeout_secs?: number;
  status?: string;
}

export interface SessionListItem {
  session_id: string;
  agent_id?: string;
  message_count?: number;
  context_window_tokens?: number;
  total_tokens?: number;
  input_tokens?: number;
  output_tokens?: number;
  cost_usd?: number;
  duration_ms?: number;
  created_at?: string;
  label?: string | null;
  active?: boolean;
}

export interface SessionDetailResponse {
  session_id?: string;
  agent_id?: string;
  message_count?: number;
  context_window_tokens?: number;
  label?: string | null;
  messages?: AgentSessionMessage[];
  created_at?: string;
}

export interface MemoryItem {
  id: string;
  content?: string;
  level?: string;
  category?: string | null;
  metadata?: Record<string, unknown>;
  created_at?: string;
  source?: string;
  confidence?: number;
  accessed_at?: string;
  access_count?: number;
  agent_id?: string;
}

export interface MemoryListResponse {
  memories?: MemoryItem[];
  total?: number;
  offset?: number;
  limit?: number;
  // Server signals whether proactive memory is enabled in config so the
  // dashboard can render an explanatory note + fall back to per-agent KV.
  proactive_enabled?: boolean;
}

export interface MemoryStatsResponse {
  total?: number;
  user_count?: number;
  session_count?: number;
  agent_count?: number;
  categories?: Record<string, number>;
  enabled?: boolean;
  auto_memorize_enabled?: boolean;
  auto_retrieve_enabled?: boolean;
  llm_extraction?: boolean;
  // Mirrors MemoryListResponse — see field doc above.
  proactive_enabled?: boolean;
}

// Per-agent KV pair returned by `GET /api/memory/agents/:id/kv`.
//
// `created_at` and `source` are best-effort: the underlying substrate may not
// populate them today, so the dashboard treats them as optional.
export interface AgentKvPair {
  key: string;
  value: unknown;
  source?: string;
  created_at?: string;
}

export interface AgentKvResponse {
  kv_pairs?: AgentKvPair[];
}

export interface UsageSummaryResponse {
  total_input_tokens?: number;
  total_output_tokens?: number;
  total_cost_usd?: number;
  call_count?: number;
  total_tool_calls?: number;
}

export interface UsageByModelItem {
  model?: string;
  total_cost_usd?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  call_count?: number;
}

export interface ModelPerformanceItem {
  model?: string;
  total_cost_usd?: number;
  total_input_tokens?: number;
  total_output_tokens?: number;
  call_count?: number;
  avg_latency_ms?: number;
  min_latency_ms?: number;
  max_latency_ms?: number;
  cost_per_call?: number;
  avg_latency_per_call?: number;
}

export interface UsageByAgentItem {
  agent_id?: string;
  name?: string;
  total_tokens?: number;
  tool_calls?: number;
  cost?: number;
}

export interface UsageDailyItem {
  date?: string;
  cost_usd?: number;
  tokens?: number;
  calls?: number;
}

export interface UsageDailyResponse {
  days?: UsageDailyItem[];
  today_cost_usd?: number;
  first_event_date?: string | null;
}

export interface CommsNode {
  id: string;
  name?: string;
  state?: string;
  model?: string;
}

export interface CommsEdge {
  from?: string;
  to?: string;
  kind?: string;
}

export interface CommsTopology {
  nodes?: CommsNode[];
  edges?: CommsEdge[];
}

export interface CommsEventItem {
  id?: string;
  timestamp?: string;
  kind?: string;
  source_id?: string;
  source_name?: string;
  target_id?: string;
  target_name?: string;
  detail?: string;
}

export interface HandRequirementItem {
  key?: string;
  label?: string;
  satisfied?: boolean;
  optional?: boolean;
  type?: string;
  description?: string;
  current_value?: string;
}

export interface HandDefinitionItem {
  id: string;
  name?: string;
  description?: string;
  category?: string;
  icon?: string;
  tools?: string[];
  requirements_met?: boolean;
  active?: boolean;
  degraded?: boolean;
  requirements?: HandRequirementItem[];
  dashboard_metrics?: number;
  has_settings?: boolean;
  settings_count?: number;
  /** True when the hand was installed by the user (lives under
   *  `home/workspaces/{id}`). Built-in hands shipped by librefang-registry
   *  report false and cannot be uninstalled. */
  is_custom?: boolean;
}

export interface HandInstanceItem {
  instance_id: string;
  hand_id?: string;
  hand_name?: string;
  hand_icon?: string;
  status?: string;
  agent_id?: string;
  agent_name?: string;
  agent_ids?: Record<string, string>;
  coordinator_role?: string;
  activated_at?: string;
  updated_at?: string;
}

export interface HandStatsResponse {
  instance_id?: string;
  hand_id?: string;
  status?: string;
  agent_id?: string;
  metrics?: Record<string, { value?: unknown; format?: string }>;
}

export interface GoalItem {
  id: string;
  title?: string;
  description?: string;
  parent_id?: string;
  agent_id?: string;
  status?: string;
  progress?: number;
  created_at?: string;
  updated_at?: string;
}

type Json = Record<string, unknown>;
const DEFAULT_POST_TIMEOUT_MS = 60_000;
const LONG_RUNNING_TIMEOUT_MS = 300_000;

// Global 401 handler — set by App.tsx to trigger login screen
let _onUnauthorized: (() => void) | null = null;
let _unauthorizedFired = false;
export function setOnUnauthorized(fn: (() => void) | null) {
  _onUnauthorized = fn;
  _unauthorizedFired = false;
}

export function getStoredApiKey(): string {
  // #3620: Prefer sessionStorage (tab-scoped, not persisted to disk) over
  // localStorage to reduce the XSS exfil window. Fall back to localStorage so
  // tokens stored by older versions of the dashboard keep working.
  return (
    sessionStorage.getItem("librefang-api-key") ||
    localStorage.getItem("librefang-api-key") ||
    ""
  );
}

export function authHeader(): HeadersInit {
  const lang = localStorage.getItem("i18nextLng") || navigator.language || "en";
  const token = getStoredApiKey();
  const headers: HeadersInit = { "Accept-Language": lang };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  return headers;
}

function buildHeaders(headers?: HeadersInit): Headers {
  const merged = new Headers(headers);
  const auth = new Headers(authHeader());
  auth.forEach((value, key) => {
    merged.set(key, value);
  });
  return merged;
}

/**
 * Build an authenticated WebSocket connection (#3620, #3963).
 *
 * The token is passed via the `Sec-WebSocket-Protocol` header using a
 * `bearer.<token>` sub-protocol instead of a `?token=` query parameter, so the
 * token never appears in URLs, server access logs, browser history, or
 * Referer headers.  The server reads the token from the first sub-protocol
 * entry that starts with `bearer.` and echoes it back in the upgrade
 * response (browsers reject the handshake otherwise).
 *
 * Returns `{ url, protocols }` — pass both to `new WebSocket(url, protocols)`.
 */
export function buildAuthenticatedWebSocket(path: string): {
  url: string;
  protocols: string[];
} {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${window.location.host}${path}`;
  const token = getStoredApiKey();
  const protocols = token ? [`bearer.${token}`] : [];
  return { url, protocols };
}

async function parseError(response: Response): Promise<ApiError> {
  // If 401, trigger global logout (only once to prevent infinite loop)
  if (response.status === 401 && _onUnauthorized && !_unauthorizedFired) {
    _unauthorizedFired = true;
    clearApiKey();
    _onUnauthorized();
  }
  const text = await response.text();
  let message = response.statusText;
  let code = `HTTP_${response.status}`;
  try {
    const json = JSON.parse(text) as Json;
    // Prefer the human-readable `detail` field over the machine-code `error` field
    if (typeof json.detail === "string") {
      message = json.detail;
    } else if (typeof json.error === "string") {
      message = json.error;
      code = json.error;
    }
  } catch {
    // ignore parse errors
  }
  return new ApiError(response.status, code, message || `HTTP ${response.status}`);
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: buildHeaders({
      "Content-Type": "application/json",
    })
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

async function post<T>(
  path: string,
  body: unknown,
  timeout = DEFAULT_POST_TIMEOUT_MS,
  externalSignal?: AbortSignal,
): Promise<T> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeout);

  // Forward external aborts (e.g. component unmount) into our controller so
  // the fetch is actually cancelled, not just the awaited promise.
  const onExternalAbort = () => controller.abort();
  if (externalSignal) {
    if (externalSignal.aborted) controller.abort();
    else externalSignal.addEventListener("abort", onExternalAbort, { once: true });
  }

  try {
    const response = await fetch(path, {
      method: "POST",
      headers: buildHeaders({
        "Content-Type": "application/json",
      }),
      body: JSON.stringify(body),
      signal: controller.signal
    });
    clearTimeout(timeoutId);
    if (!response.ok) {
      throw await parseError(response);
    }
    return (await response.json()) as T;
  } catch (error) {
    clearTimeout(timeoutId);
    if (externalSignal?.aborted) {
      // Re-throw as DOMException so callers can identify caller-initiated aborts.
      throw new DOMException("Aborted", "AbortError");
    }
    if (error instanceof Error && error.name === "AbortError") {
      throw new Error(`Request timeout after ${Math.round(timeout / 1000)}s - operation may still be running`);
    }
    throw error;
  } finally {
    if (externalSignal) {
      externalSignal.removeEventListener("abort", onExternalAbort);
    }
  }
}

async function put<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "PUT",
    headers: buildHeaders({
      "Content-Type": "application/json",
    }),
    body: JSON.stringify(body)
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

async function patch<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "PATCH",
    headers: buildHeaders({
      "Content-Type": "application/json",
    }),
    body: JSON.stringify(body)
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

async function del<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    method: "DELETE",
    headers: buildHeaders({
      "Content-Type": "application/json",
    })
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

async function getText(path: string): Promise<string> {
  const response = await fetch(path, {
    headers: buildHeaders(),
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return response.text();
}

export async function postQuickInit(): Promise<{ status: string; provider?: string; model?: string; message?: string }> {
  return post("/api/init", {});
}

export async function loadDashboardSnapshot(): Promise<DashboardSnapshot> {
  const snap = await get<{
    health: HealthResponse;
    status: StatusResponse;
    agents: AgentItem[];
    providers: ProviderItem[];
    channels: ChannelItem[];
    skillCount: number;
    workflowCount: number;
    webSearchAvailable: boolean;
  }>("/api/dashboard/snapshot");

  return {
    health: snap.health,
    status: snap.status,
    agents: snap.agents ?? [],
    providers: snap.providers ?? [],
    channels: snap.channels ?? [],
    skillCount: snap.skillCount ?? 0,
    workflowCount: snap.workflowCount ?? 0,
    webSearchAvailable: snap.webSearchAvailable ?? false,
  };
}


export interface AgentModelDetail {
  provider?: string;
  model?: string;
  max_tokens?: number;
  temperature?: number;
}

export interface AgentDetail {
  id: string;
  name: string;
  model?: AgentModelDetail;
  system_prompt?: string;
  capabilities?: { tools?: boolean; network?: boolean };
  skills?: string[];
  tags?: string[];
  mode?: string;
  thinking?: { budget_tokens?: number; stream_thinking?: boolean };
  is_hand?: boolean;
  web_search_augmentation?: "off" | "auto" | "always";
}

export async function getAgentDetail(agentId: string): Promise<AgentDetail> {
  return get<AgentDetail>(`/api/agents/${encodeURIComponent(agentId)}`);
}

/** 24-hour KPI rollup for one agent — backs the AgentsPage detail-panel
 *  KPI tiles. See `GET /api/agents/{id}/stats`. */
export interface AgentStats24h {
  sessions_24h: number;
  cost_24h: number;
  p95_latency_ms: number;
  active_now: number;
  samples: number;
  /** Same window-scoped fields, aggregated over the prior 24h (24-48h
   *  ago). Optional so older backends that don't ship the field don't
   *  break the type at runtime — the dashboard already gates on
   *  `live?.prev` and falls back to non-delta subtext. */
  prev?: {
    sessions_24h: number;
    cost_24h: number;
    p95_latency_ms: number;
  };
}

export async function getAgentStats(agentId: string): Promise<AgentStats24h> {
  return get<AgentStats24h>(`/api/agents/${encodeURIComponent(agentId)}/stats`);
}

export async function patchAgentConfig(
  agentId: string,
  config: {
    max_tokens?: number;
    model?: string;
    provider?: string;
    temperature?: number;
    web_search_augmentation?: "off" | "auto" | "always";
  },
): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>(
    `/api/agents/${encodeURIComponent(agentId)}/config`,
    config,
  );
}

function trimOptionalHandRuntimeString(value: string | undefined): string | undefined {
  if (value === undefined) {
    return undefined;
  }
  return value.trim();
}

/**
 * Hand runtime PATCH is the only agent-config write path with tri-state string
 * semantics: absent leaves the override untouched, empty string clears it.
 * Keep this serializer scoped to `/hand-runtime-config` so other PATCH payloads
 * do not silently inherit those semantics.
 */
function serializeHandAgentRuntimeConfigPatch(config: {
  max_tokens?: number;
  model?: string;
  provider?: string;
  temperature?: number;
  api_key_env?: string;
  base_url?: string;
  web_search_augmentation?: "off" | "auto" | "always";
}): {
  max_tokens?: number;
  model?: string;
  provider?: string;
  temperature?: number;
  api_key_env?: string;
  base_url?: string;
  web_search_augmentation?: "off" | "auto" | "always";
} {
  return {
    ...config,
    api_key_env: trimOptionalHandRuntimeString(config.api_key_env),
    base_url: trimOptionalHandRuntimeString(config.base_url),
  };
}

/** PATCH /api/agents/{id}/hand-runtime-config — partial update of per-agent
 * hand runtime overrides. Empty string for `api_key_env` / `base_url` clears
 * that specific field (tri-state: absent = leave as-is, empty = clear,
 * value = set). Distinct from `/agents/{id}/config` which targets the
 * standalone agent config path. */
export async function patchHandAgentRuntimeConfig(
  agentId: string,
  config: {
    max_tokens?: number;
    model?: string;
    provider?: string;
    temperature?: number;
    api_key_env?: string;
    base_url?: string;
    web_search_augmentation?: "off" | "auto" | "always";
  },
): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>(
    `/api/agents/${encodeURIComponent(agentId)}/hand-runtime-config`,
    serializeHandAgentRuntimeConfigPatch(config),
  );
}

/** DELETE /api/agents/{id}/hand-runtime-config — drop all per-agent runtime
 * overrides for the hand role, restoring the live manifest to the HAND.toml
 * defaults. The server returns 204 No Content on success, so we bypass the
 * shared `del<T>` helper (which assumes a JSON body) and handle the empty
 * response explicitly. */
export async function clearHandAgentRuntimeConfig(agentId: string): Promise<void> {
  const response = await fetch(
    `/api/agents/${encodeURIComponent(agentId)}/hand-runtime-config`,
    {
      method: "DELETE",
      headers: buildHeaders({
        "Content-Type": "application/json",
      }),
    },
  );
  if (!response.ok) {
    throw await parseError(response);
  }
}

/** PATCH /api/agents/{id} — manifest-level partial updates (name, description,
 * system_prompt, mcp_servers, model). Distinct from `/agents/{id}/config`
 * which only accepts the model-tuning subset. */
export async function patchAgent(agentId: string, body: { name?: string; description?: string; system_prompt?: string; model?: string; provider?: string; mcp_servers?: string[] }): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}`, body);
}

export interface AgentToolsResponse {
  capabilities_tools?: string[] | null;
  tool_allowlist?: string[] | null;
  tool_blocklist?: string[] | null;
  disabled?: boolean;
}

export interface ToolDefinition {
  name: string;
  description?: string;
}

export async function getAgentTools(agentId: string): Promise<AgentToolsResponse> {
  return get<AgentToolsResponse>(`/api/agents/${encodeURIComponent(agentId)}/tools`);
}

export async function updateAgentTools(agentId: string, payload: { capabilities_tools?: string[]; tool_allowlist?: string[]; tool_blocklist?: string[] }): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/tools`, payload);
}

export async function listAgents(
  opts: { includeHands?: boolean } = {},
): Promise<AgentItem[]> {
  const params = new URLSearchParams({
    limit: "500",
    sort: "last_active",
    order: "desc",
  });
  if (opts.includeHands) {
    params.set("include_hands", "true");
  }
  const data = await get<PaginatedResponse<AgentItem>>(
    `/api/agents?${params.toString()}`,
  );
  return data.items ?? [];
}

export interface AgentTemplate {
  name: string;
  description: string;
}

export async function listAgentTemplates(): Promise<AgentTemplate[]> {
  const data = await get<{ templates: AgentTemplate[] }>("/api/templates");
  return data.templates ?? [];
}

export async function getAgentTemplateToml(name: string): Promise<string> {
  return getText(`/api/templates/${encodeURIComponent(name)}/toml`);
}

export async function deleteAgent(agentId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}`);
}

export async function cloneAgent(agentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/clone`, {});
}

export async function stopAgent(agentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/stop`, {});
}

export async function clearAgentHistory(agentId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/history`);
}

export async function resetAgentSession(agentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/reset`, {});
}

export async function loadAgentSession(
  agentId: string,
  sessionId?: string | null,
): Promise<AgentSessionResponse> {
  const qs = sessionId ? `?session_id=${encodeURIComponent(sessionId)}` : "";
  return get<AgentSessionResponse>(`/api/agents/${encodeURIComponent(agentId)}/session${qs}`);
}

export async function sendAgentMessage(
  agentId: string,
  message: string,
  options?: SendAgentMessageOptions,
): Promise<AgentMessageResponse> {
  const body: Record<string, unknown> = { message };
  if (options?.thinking !== undefined) body.thinking = options.thinking;
  if (options?.show_thinking !== undefined) body.show_thinking = options.show_thinking;
  if (options?.session_id) body.session_id = options.session_id;
  if (options?.attachments && options.attachments.length > 0) body.attachments = options.attachments;
  return post<AgentMessageResponse>(
    `/api/agents/${encodeURIComponent(agentId)}/message`,
    body,
    LONG_RUNNING_TIMEOUT_MS,
  );
}

export async function listProviders(): Promise<ProviderItem[]> {
  const data = await get<ProvidersResponse>("/api/providers");
  return data.providers ?? [];
}

export async function testProvider(providerId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/test`, {});
}

export interface ModelItem {
  id: string;
  display_name?: string;
  provider: string;
  tier?: string;
  context_window?: number;
  max_output_tokens?: number;
  input_cost_per_m?: number;
  output_cost_per_m?: number;
  supports_tools?: boolean;
  supports_vision?: boolean;
  supports_streaming?: boolean;
  supports_thinking?: boolean;
  aliases?: string[];
  available?: boolean;
}

export async function listModels(params?: { provider?: string; tier?: string; available?: boolean }): Promise<{ models: ModelItem[]; total: number; available: number }> {
  const query = new URLSearchParams();
  if (params?.provider) query.set("provider", params.provider);
  if (params?.tier) query.set("tier", params.tier);
  if (params?.available !== undefined) query.set("available", String(params.available));
  const qs = query.toString();
  return get<{ models: ModelItem[]; total: number; available: number }>(`/api/models${qs ? `?${qs}` : ""}`);
}

export async function addCustomModel(model: {
  id: string;
  provider: string;
  display_name?: string;
  context_window?: number;
  max_output_tokens?: number;
  input_cost_per_m?: number;
  output_cost_per_m?: number;
  supports_tools?: boolean;
  supports_vision?: boolean;
  supports_streaming?: boolean;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/models/custom", model);
}

export async function removeCustomModel(modelId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/models/custom/${encodeURIComponent(modelId)}`);
}

// ── Per-model overrides ─────────────────────────────────────────

export interface ModelOverrides {
  model_type?: "chat" | "speech" | "embedding";
  temperature?: number;
  top_p?: number;
  max_tokens?: number;
  frequency_penalty?: number;
  presence_penalty?: number;
  reasoning_effort?: string;
  use_max_completion_tokens?: boolean;
  no_system_role?: boolean;
  force_max_tokens?: boolean;
}

export async function getModelOverrides(modelKey: string): Promise<ModelOverrides> {
  return get<ModelOverrides>(`/api/models/overrides/${encodeURIComponent(modelKey)}`);
}

export async function updateModelOverrides(modelKey: string, overrides: ModelOverrides): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/models/overrides/${encodeURIComponent(modelKey)}`, overrides);
}

export async function deleteModelOverrides(modelKey: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/models/overrides/${encodeURIComponent(modelKey)}`);
}

export async function setProviderKey(providerId: string, key: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/key`, { key });
}

export async function deleteProviderKey(providerId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/key`);
}

export async function setProviderUrl(providerId: string, baseUrl: string, proxyUrl?: string): Promise<ApiActionResponse> {
  const body: Record<string, string> = { base_url: baseUrl };
  if (proxyUrl !== undefined) body.proxy_url = proxyUrl;
  return put<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/url`, body);
}

export async function setDefaultProvider(providerId: string, model?: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/default`, model ? { model } : {});
}

// ── Media generation API ──────────────────────────────────────────────

export async function listMediaProviders(): Promise<MediaProvider[]> {
  const data = await get<{ providers: MediaProvider[] }>("/api/media/providers");
  return data.providers ?? [];
}

export async function generateImage(req: { prompt: string; provider?: string; model?: string; count?: number; aspect_ratio?: string }): Promise<MediaImageResult> {
  return post<MediaImageResult>("/api/media/image", req);
}

export interface SpeechResult {
  url: string;
  format: string;
  provider: string;
  model: string;
  duration_ms?: number;
  sample_rate?: number;
}

export async function synthesizeSpeech(req: { text: string; provider?: string; model?: string; voice?: string; format?: string; language?: string; speed?: number }): Promise<SpeechResult> {
  return post<SpeechResult>("/api/media/speech", req);
}

export async function transcribeAudio(audioBlob: Blob): Promise<{ text: string; provider: string; model: string }> {
  const response = await fetch("/api/media/transcribe", {
    method: "POST",
    headers: buildHeaders({
      "Content-Type": audioBlob.type || "audio/webm",
    }),
    body: audioBlob,
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as { text: string; provider: string; model: string };
}

// HTTP header values must be visible-ASCII (RFC 7230). Browsers reject
// non-ASCII bytes in fetch headers, so we replace anything outside
// 0x20–0x7e (and the header-breaking quote/CR/LF) with `_` before sending
// the original filename. This loses fidelity for unicode names but never
// throws, and keeps the server-side label render-safe — no decode pass
// needed at display time.
function sanitizeFilenameForHeader(name: string): string {
  // eslint-disable-next-line no-control-regex
  return name.replace(/[^\x20-\x7e]|["\r\n]/g, "_");
}

// Upload a chat attachment for an agent. Body is the raw file bytes; backend
// expects `Content-Type` to match the file MIME and `X-Filename` for the
// original name. Server-side limits: 10MB and an exact MIME allowlist
// (image/audio/text/pdf) — callers should still pre-validate to fail fast.
export async function uploadAgentFile(agentId: string, file: File): Promise<AgentFileUploadResult> {
  const response = await fetch(`/api/agents/${encodeURIComponent(agentId)}/upload`, {
    method: "POST",
    headers: buildHeaders({
      "Content-Type": file.type || "application/octet-stream",
      "X-Filename": sanitizeFilenameForHeader(file.name),
    }),
    body: file,
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as AgentFileUploadResult;
}

export async function submitVideo(req: { prompt: string; provider?: string; model?: string }): Promise<MediaVideoSubmitResult> {
  return post<MediaVideoSubmitResult>("/api/media/video", req);
}

export async function pollVideo(taskId: string, provider: string): Promise<MediaVideoStatus> {
  return get<MediaVideoStatus>(`/api/media/video/${encodeURIComponent(taskId)}?provider=${encodeURIComponent(provider)}`);
}

export async function generateMusic(req: { prompt?: string; lyrics?: string; provider?: string; model?: string; instrumental?: boolean }): Promise<MediaMusicResult> {
  return post<MediaMusicResult>("/api/media/music", req);
}

export async function listChannels(): Promise<ChannelItem[]> {
  const data = await get<ChannelsResponse>("/api/channels");
  return data.channels ?? [];
}

export async function testChannel(channelName: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/channels/${encodeURIComponent(channelName)}/test`, {});
}

export async function configureChannel(channelName: string, config: Record<string, unknown>): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/channels/${encodeURIComponent(channelName)}/configure`, { fields: config });
}

export async function reloadChannels(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/channels/reload", {});
}

export interface QrStartResponse {
  available: boolean;
  qr_code?: string;
  qr_url?: string;
  message?: string;
}

export interface QrStatusResponse {
  connected: boolean;
  expired: boolean;
  message?: string;
  bot_token?: string;
}

export async function wechatQrStart(): Promise<QrStartResponse> {
  return post<QrStartResponse>("/api/channels/wechat/qr/start", {});
}

export async function wechatQrStatus(qrCode: string): Promise<QrStatusResponse> {
  return get<QrStatusResponse>(`/api/channels/wechat/qr/status?qr_code=${encodeURIComponent(qrCode)}`);
}

export async function whatsappQrStart(): Promise<QrStartResponse> {
  return post<QrStartResponse>("/api/channels/whatsapp/qr/start", {});
}

export async function whatsappQrStatus(qrCode: string): Promise<QrStatusResponse> {
  return get<QrStatusResponse>(`/api/channels/whatsapp/qr/status?qr_code=${encodeURIComponent(qrCode)}`);
}

export async function listSkills(): Promise<SkillItem[]> {
  const data = await get<SkillsResponse>("/api/skills");
  return data.skills ?? [];
}

export async function listTools(): Promise<ToolDefinition[]> {
  const data = await get<{ tools?: ToolDefinition[] } | ToolDefinition[]>("/api/tools");
  if (Array.isArray(data)) return data;
  return data.tools ?? [];
}

export async function installSkill(name: string, hand?: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skills/install", { name, hand }, LONG_RUNNING_TIMEOUT_MS);
}

export async function uninstallSkill(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skills/uninstall", { name });
}

// Skill evolution APIs
export async function getSkillDetail(name: string): Promise<SkillDetail> {
  return get<SkillDetail>(`/api/skills/${encodeURIComponent(name)}`);
}

export async function createSkill(
  params: {
    name: string;
    description: string;
    prompt_context: string;
    tags?: string[];
  },
  signal?: AbortSignal,
): Promise<EvolutionResult> {
  return post<EvolutionResult>("/api/skills/create", params, undefined, signal);
}

export async function reloadSkills(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skills/reload", {});
}

// Skill evolution mutation APIs (dashboard Update/Patch/Rollback/Files flow)
export async function evolveUpdateSkill(name: string, params: {
  prompt_context: string;
  changelog: string;
}): Promise<EvolutionResult> {
  return post<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/update`, params);
}

export async function evolvePatchSkill(name: string, params: {
  old_string: string;
  new_string: string;
  changelog: string;
  replace_all?: boolean;
}): Promise<EvolutionResult> {
  return post<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/patch`, params);
}

export async function evolveRollbackSkill(name: string): Promise<EvolutionResult> {
  return post<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/rollback`, {});
}

export async function evolveDeleteSkill(name: string): Promise<EvolutionResult> {
  return post<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/delete`, {});
}

export async function evolveWriteFile(name: string, params: {
  path: string;
  content: string;
}): Promise<EvolutionResult> {
  return post<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/file`, params);
}

export async function evolveRemoveFile(name: string, path: string): Promise<EvolutionResult> {
  return del<EvolutionResult>(`/api/skills/${encodeURIComponent(name)}/evolve/file?path=${encodeURIComponent(path)}`);
}

export interface SupportingFileContents {
  name: string;
  path: string;
  content: string;
  truncated: boolean;
}

export async function getSupportingFile(name: string, path: string): Promise<SupportingFileContents> {
  return get<SupportingFileContents>(`/api/skills/${encodeURIComponent(name)}/file?path=${encodeURIComponent(path)}`);
}

// ClawHub types
export interface ClawHubBrowseItem {
  slug: string;
  name: string;
  description: string;
  version: string;
  author?: string;
  stars?: number;
  downloads?: number;
  tags?: string[];
  icon_url?: string;
  updated_at?: number;
  score?: number;
}

export interface ClawHubBrowseResponse {
  items: ClawHubBrowseItem[];
  next_cursor?: string;
}

export interface ClawHubSkillDetail {
  slug: string;
  name: string;
  description: string;
  version: string;
  author: string;
  stars: number;
  downloads: number;
  tags: string[];
  readme: string;
  icon_url?: string;
  is_installed?: boolean;
  installed?: boolean;
}

// ClawHub API
export async function clawhubBrowse(sort?: string, limit?: number, cursor?: string): Promise<ClawHubBrowseResponse> {
  const params = new URLSearchParams();
  if (sort) params.set("sort", sort);
  if (limit) params.set("limit", String(limit));
  if (cursor) params.set("cursor", cursor);
  return get<ClawHubBrowseResponse>(`/api/clawhub/browse?${params}`);
}

export async function clawhubSearch(query: string): Promise<ClawHubBrowseResponse> {
  return get<ClawHubBrowseResponse>(`/api/clawhub/search?q=${encodeURIComponent(query)}`);
}

export async function clawhubGetSkill(slug: string): Promise<ClawHubSkillDetail> {
  return get<ClawHubSkillDetail>(`/api/clawhub/skill/${encodeURIComponent(slug)}`);
}

export async function clawhubInstall(slug: string, version?: string, hand?: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(
    "/api/clawhub/install",
    { slug, version: version || "latest", hand },
    LONG_RUNNING_TIMEOUT_MS
  );
}

// ── ClawHub China mirror API (mirror-cn.clawhub.com) ──

export async function clawhubCnBrowse(sort?: string, limit?: number, cursor?: string): Promise<ClawHubBrowseResponse> {
  const params = new URLSearchParams();
  if (sort) params.set("sort", sort);
  if (limit) params.set("limit", String(limit));
  if (cursor) params.set("cursor", cursor);
  return get<ClawHubBrowseResponse>(`/api/clawhub-cn/browse?${params}`);
}

export async function clawhubCnSearch(query: string): Promise<ClawHubBrowseResponse> {
  return get<ClawHubBrowseResponse>(`/api/clawhub-cn/search?q=${encodeURIComponent(query)}`);
}

export async function clawhubCnGetSkill(slug: string): Promise<ClawHubSkillDetail> {
  return get<ClawHubSkillDetail>(`/api/clawhub-cn/skill/${encodeURIComponent(slug)}`);
}

export async function clawhubCnInstall(slug: string, version?: string, hand?: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(
    "/api/clawhub-cn/install",
    { slug, version: version || "latest", hand },
    LONG_RUNNING_TIMEOUT_MS
  );
}

// ── Skillhub API ─────────────────────────────────────

export async function skillhubSearch(query: string): Promise<ClawHubBrowseResponse> {
  return get<ClawHubBrowseResponse>(`/api/skillhub/search?q=${encodeURIComponent(query)}&limit=20`);
}

export async function skillhubBrowse(sort?: string): Promise<ClawHubBrowseResponse> {
  const params = new URLSearchParams();
  if (sort) params.set("sort", sort);
  params.set("limit", "50");
  return get<ClawHubBrowseResponse>(`/api/skillhub/browse?${params}`);
}

export async function skillhubInstall(slug: string, hand?: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skillhub/install", { slug, hand }, LONG_RUNNING_TIMEOUT_MS);
}

export async function skillhubGetSkill(slug: string): Promise<ClawHubSkillDetail> {
  return get<ClawHubSkillDetail>(`/api/skillhub/skill/${encodeURIComponent(slug)}`);
}

// ── FangHub (official LibreFang registry skills) ──────

export interface FangHubSkill {
  name: string;
  description: string;
  version: string;
  author?: string;
  tags?: string[];
  is_installed: boolean;
}

export interface FangHubListResponse {
  skills: FangHubSkill[];
  total: number;
}

export async function fanghubListSkills(): Promise<FangHubListResponse> {
  return get<FangHubListResponse>("/api/skills/registry");
}

// ── Workflow Templates ────────────────────────────────

export interface TemplateParameter {
  name: string;
  description?: string;
  param_type?: string;
  default?: unknown;
  required?: boolean;
}

export interface TemplateI18n {
  name?: string;
  description?: string;
}

export interface WorkflowTemplate {
  id: string;
  name: string;
  description?: string;
  category?: string;
  tags?: string[];
  parameters?: TemplateParameter[];
  steps?: WorkflowStep[];
  i18n?: Record<string, TemplateI18n>;
}

export async function listWorkflowTemplates(q?: string, category?: string): Promise<WorkflowTemplate[]> {
  const params = new URLSearchParams();
  if (q) params.set("q", q);
  if (category) params.set("category", category);
  const qs = params.toString();
  const data = await get<{ templates?: WorkflowTemplate[] }>(`/api/workflow-templates${qs ? `?${qs}` : ""}`);
  return data.templates ?? [];
}

export async function getWorkflowTemplate(id: string): Promise<WorkflowTemplate> {
  return get<WorkflowTemplate>(`/api/workflow-templates/${encodeURIComponent(id)}`);
}

export async function instantiateTemplate(id: string, params: Record<string, unknown>): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/workflow-templates/${encodeURIComponent(id)}/instantiate`, params);
}

export async function listWorkflows(): Promise<WorkflowItem[]> {
  const data = await get<{ workflows?: WorkflowItem[] }>("/api/workflows");
  return data.workflows ?? [];
}

export async function createWorkflow(payload: {
  name: string;
  description?: string;
  steps: Array<{
    name: string;
    agent_name?: string;
    agent_id?: string;
    prompt: string;
    timeout_secs?: number;
  }>;
  layout?: unknown;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/workflows", payload).then((r) => ({
    ...r,
    id: r.id ?? (r as { workflow_id?: string }).workflow_id,
  }));
}

export async function getWorkflow(workflowId: string): Promise<WorkflowItem> {
  return get<WorkflowItem>(`/api/workflows/${encodeURIComponent(workflowId)}`);
}

export async function runWorkflow(workflowId: string, input: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}/run`, {
    input
  }, LONG_RUNNING_TIMEOUT_MS); // 5 min timeout — workflows run multiple LLM steps
}

export async function deleteWorkflow(workflowId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}`);
}

export async function updateWorkflow(workflowId: string, payload: {
  name?: string;
  description?: string;
  steps?: Array<{
    name: string;
    agent_name?: string;
    agent_id?: string;
    prompt: string;
    timeout_secs?: number;
  }>;
  layout?: unknown;
}): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}`, payload);
}

export async function listWorkflowRuns(workflowId: string): Promise<WorkflowRunItem[]> {
  return get<WorkflowRunItem[]>(`/api/workflows/${encodeURIComponent(workflowId)}/runs`);
}

/** Per-step execution result returned by run/detail endpoints. */
export interface WorkflowStepResult {
  step_name: string;
  agent_id?: string;
  agent_name: string;
  /** The actual prompt sent to the agent after variable expansion. */
  prompt: string;
  output: string;
  input_tokens: number;
  output_tokens: number;
  duration_ms: number;
}

/** Full detail for a single workflow run. */
export interface WorkflowRunDetail {
  id: string;
  workflow_id: string;
  workflow_name: string;
  input: string;
  state: string;
  output?: string;
  error?: string;
  started_at: string;
  completed_at?: string | null;
  step_results: WorkflowStepResult[];
}

/** Per-step preview returned by dry-run. */
export interface DryRunStepPreview {
  step_name: string;
  agent_name?: string;
  agent_found: boolean;
  resolved_prompt: string;
  skipped: boolean;
  skip_reason?: string;
}

/** Response from the dry-run endpoint. */
export interface DryRunResult {
  valid: boolean;
  steps: DryRunStepPreview[];
}

/**
 * Validate a workflow without making any LLM calls.
 * Returns per-step previews with resolved prompts and agent resolution status.
 */
export async function dryRunWorkflow(workflowId: string, input: string): Promise<DryRunResult> {
  return post<DryRunResult>(
    `/api/workflows/${encodeURIComponent(workflowId)}/dry-run`,
    { input },
    30000
  );
}

/** Fetch full detail for a single workflow run (includes step-level I/O). */
export async function getWorkflowRun(runId: string): Promise<WorkflowRunDetail> {
  return get<WorkflowRunDetail>(`/api/workflows/runs/${encodeURIComponent(runId)}`);
}

export async function saveWorkflowAsTemplate(workflowId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}/save-as-template`, {});
}

export async function listSchedules(): Promise<ScheduleItem[]> {
  const data = await get<{ schedules?: ScheduleItem[]; total?: number }>("/api/schedules");
  return data.schedules ?? [];
}

export async function createSchedule(payload: {
  name: string;
  cron: string;
  tz?: string;
  agent_id?: string;
  workflow_id?: string;
  message?: string;
  enabled?: boolean;
  /** Fan-out destinations. Empty array clears any existing list on update. */
  delivery_targets?: CronDeliveryTarget[];
}): Promise<ScheduleItem> {
  return post<ScheduleItem>("/api/schedules", payload);
}

export async function updateSchedule(
  scheduleId: string,
  payload: {
    enabled?: boolean;
    name?: string;
    cron?: string;
    tz?: string;
    agent_id?: string;
    message?: string;
    /**
     * Replace fan-out delivery targets. The backend treats this as a full
     * replace (an empty array clears the list). Omit the field to leave it
     * unchanged.
     */
    delivery_targets?: CronDeliveryTarget[];
  }
): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/schedules/${encodeURIComponent(scheduleId)}`, payload);
}

export async function deleteSchedule(scheduleId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/schedules/${encodeURIComponent(scheduleId)}`);
}

export async function runSchedule(scheduleId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/schedules/${encodeURIComponent(scheduleId)}/run`, {});
}

export async function listTriggers(): Promise<TriggerItem[]> {
  const data = await get<{ triggers?: TriggerItem[] }>("/api/triggers");
  return data.triggers ?? [];
}

export async function getTrigger(triggerId: string): Promise<TriggerItem> {
  return get<TriggerItem>(`/api/triggers/${encodeURIComponent(triggerId)}`);
}

export async function createTrigger(
  payload: CreateTriggerPayload
): Promise<ApiActionResponse & { trigger_id?: string }> {
  return post<ApiActionResponse & { trigger_id?: string }>("/api/triggers", payload);
}

export async function updateTrigger(
  triggerId: string,
  updates: TriggerPatch
): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>(`/api/triggers/${encodeURIComponent(triggerId)}`, updates);
}

export async function deleteTrigger(triggerId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/triggers/${encodeURIComponent(triggerId)}`);
}

export async function listCronJobs(agentId?: string): Promise<CronJobItem[]> {
  const url = agentId ? `/api/cron/jobs?agent_id=${encodeURIComponent(agentId)}` : "/api/cron/jobs";
  const data = await get<{ jobs?: CronJobItem[]; total?: number }>(url);
  return data.jobs ?? [];
}

export async function getVersionInfo(): Promise<VersionResponse> {
  return get<VersionResponse>("/api/version");
}

export async function getStatus(): Promise<StatusResponse> {
  return get<StatusResponse>("/api/status");
}

export async function getQueueStatus(): Promise<QueueStatusResponse> {
  return get<QueueStatusResponse>("/api/queue/status");
}

export async function shutdownServer(): Promise<{ status: string }> {
  return post<{ status: string }>("/api/shutdown", {});
}

export async function reloadConfig(): Promise<{ status: string; restart_required?: boolean; restart_reasons?: string[] }> {
  return post<{ status: string; restart_required?: boolean; restart_reasons?: string[] }>("/api/config/reload", {});
}

export interface HealthDetailResponse {
  status?: string;
  version?: string;
  uptime_seconds?: number;
  panic_count?: number;
  restart_count?: number;
  agent_count?: number;
  database?: string;
  memory?: {
    embedding_available?: boolean;
    embedding_provider?: string;
    embedding_model?: string;
    proactive_memory_enabled?: boolean;
    extraction_model?: string;
  };
  config_warnings?: string[];
}

export interface SecurityStatusResponse {
  core_protections?: Record<string, boolean>;
  configurable?: {
    rate_limiter?: { enabled?: boolean; tokens_per_minute?: number; algorithm?: string };
    websocket_limits?: { max_per_ip?: number; idle_timeout_secs?: number; max_message_size?: number; max_messages_per_minute?: number };
    wasm_sandbox?: { fuel_metering?: boolean; epoch_interruption?: boolean; default_timeout_secs?: number; default_fuel_limit?: number };
    auth?: { mode?: string; api_key_set?: boolean };
  };
  monitoring?: {
    audit_trail?: { enabled?: boolean; algorithm?: string; entry_count?: number };
    taint_tracking?: { enabled?: boolean; tracked_labels?: string[] };
    manifest_signing?: { algorithm?: string; available?: boolean };
  };
  secret_zeroization?: boolean;
  total_features?: number;
}

export interface BackupItem {
  filename?: string;
  path?: string;
  size_bytes?: number;
  modified_at?: string;
  components?: string[];
  librefang_version?: string;
  created_at?: string;
}

export interface TaskQueueStatusResponse {
  total?: number;
  pending?: number;
  in_progress?: number;
  completed?: number;
  failed?: number;
}

export interface TaskQueueItem {
  id?: string;
  status?: string;
  created_at?: string;
  updated_at?: string;
  [key: string]: unknown;
}

export async function getHealthDetail(): Promise<HealthDetailResponse> {
  return get<HealthDetailResponse>("/api/health/detail");
}

export interface MemoryConfigResponse {
  embedding_provider?: string;
  embedding_model?: string;
  embedding_api_key_env?: string;
  decay_rate?: number;
  proactive_memory?: {
    enabled?: boolean;
    auto_memorize?: boolean;
    auto_retrieve?: boolean;
    extraction_model?: string;
    max_retrieve?: number;
  };
}

export async function getMemoryConfig(): Promise<MemoryConfigResponse> {
  return get<MemoryConfigResponse>("/api/memory/config");
}

export async function updateMemoryConfig(payload: {
  embedding_provider?: string;
  embedding_model?: string;
  embedding_api_key_env?: string;
  decay_rate?: number;
  proactive_memory?: {
    enabled?: boolean;
    auto_memorize?: boolean;
    auto_retrieve?: boolean;
    extraction_model?: string;
    max_retrieve?: number;
  };
}): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>("/api/memory/config", payload);
}

export async function getSecurityStatus(): Promise<SecurityStatusResponse> {
  return get<SecurityStatusResponse>("/api/security");
}

export async function getFullConfig(): Promise<Record<string, unknown>> {
  return get<Record<string, unknown>>("/api/config");
}

/* ------------------------------------------------------------------ */
/*  Config schema (draft-07)                                           */
/* ------------------------------------------------------------------ */
/* The backend now emits a draft-07 JSON Schema generated from the   */
/* `KernelConfig` Rust type via `schemars`, plus two extensions:     */
/*   - `x-sections`: ordered UI section groupings                    */
/*   - `x-ui-options`: per-field UI hints (min/max/step/select opts) */
/* keyed by JSON pointer (e.g. `/memory/decay_rate`).                */

/** A draft-07 JSON Schema node (partial — only the fields the UI reads). */
export interface JsonSchema {
  type?: string | string[];
  title?: string;
  description?: string;
  default?: unknown;
  enum?: unknown[];
  oneOf?: JsonSchema[];
  allOf?: JsonSchema[];
  anyOf?: JsonSchema[];
  properties?: Record<string, JsonSchema>;
  additionalProperties?: boolean | JsonSchema;
  items?: JsonSchema;
  required?: string[];
  minimum?: number;
  maximum?: number;
  multipleOf?: number;
  format?: string;
  $ref?: string;
  definitions?: Record<string, JsonSchema>;
}

/** UI-only option overrides the struct cannot carry. */
export interface UiFieldOptions {
  /** Curated select options — strings or `{value,label}` objects. */
  select?: (string | { value: string; label: string })[];
  /** `{id,name,provider}`-shaped options (the model picker). */
  select_objects?: { id: string; name: string; provider: string }[];
  /** Select whose values are numeric-strings (e.g. `["0","1","6"]`). */
  number_select?: string[];
  /** UI numeric constraints. `min`/`max` may differ from the struct's
      `#[schemars(range)]` bounds when the UI wants tighter suggestion
      limits than runtime validation. */
  min?: number;
  max?: number;
  step?: number;
  placeholder?: string;
}

export interface ConfigSectionDescriptor {
  key: string;
  title?: string;
  /** `true` → fields read from the root of `KernelConfig`. `false`/omitted →
      fields are inside `properties[struct_field]`. */
  root_level?: boolean;
  /** Name of the `KernelConfig` field holding this section's sub-struct. */
  struct_field?: string;
  /** When true, changes to this section are hot-reloaded without restart. */
  hot_reloadable?: boolean;
  /** For `root_level` sections: explicit field ordering. */
  fields?: string[];
}

export interface ConfigSchemaRoot extends JsonSchema {
  "x-sections"?: ConfigSectionDescriptor[];
  "x-ui-options"?: Record<string, UiFieldOptions>;
}

export async function getConfigSchema(): Promise<ConfigSchemaRoot> {
  return get<ConfigSchemaRoot>("/api/config/schema");
}

/** Resolve a `$ref` (e.g. `#/definitions/MemoryConfig`) in the root schema. */
export function resolveRef(
  root: ConfigSchemaRoot,
  ref: string,
): JsonSchema | undefined {
  if (!ref.startsWith("#/")) return undefined;
  const path = ref.slice(2).split("/");
  let node: unknown = root;
  for (const seg of path) {
    if (typeof node !== "object" || node === null) return undefined;
    node = (node as Record<string, unknown>)[seg];
  }
  return (node as JsonSchema) ?? undefined;
}

/** Get a property's schema with `$ref`s resolved one level. */
export function deref(
  root: ConfigSchemaRoot,
  node: JsonSchema | undefined,
): JsonSchema | undefined {
  if (!node) return undefined;
  if (node.$ref) return resolveRef(root, node.$ref);
  return node;
}

export async function setConfigValue(
  path: string,
  value: unknown,
): Promise<{ status: string; restart_required?: boolean; reload_error?: string }> {
  return post<{ status: string; restart_required?: boolean; reload_error?: string }>(
    "/api/config/set",
    { path, value },
  );
}

export async function listBackups(): Promise<{ backups?: BackupItem[]; total?: number }> {
  return get<{ backups?: BackupItem[]; total?: number }>("/api/backups");
}

export async function createBackup(): Promise<{ filename?: string; path?: string; size_bytes?: number; components?: string[]; created_at?: string }> {
  return post<{ filename?: string; path?: string; size_bytes?: number; components?: string[]; created_at?: string }>("/api/backup", {});
}

export async function restoreBackup(filename: string): Promise<{ restored_files?: number; errors?: string[]; message?: string }> {
  return post<{ restored_files?: number; errors?: string[]; message?: string }>("/api/restore", { filename });
}

export async function deleteBackup(filename: string): Promise<{ deleted?: string }> {
  return del<{ deleted?: string }>(`/api/backups/${encodeURIComponent(filename)}`);
}

export async function getTaskQueueStatus(): Promise<TaskQueueStatusResponse> {
  return get<TaskQueueStatusResponse>("/api/tasks/status");
}

export async function listTaskQueue(status?: string): Promise<{ tasks?: TaskQueueItem[]; total?: number }> {
  const qs = status ? `?status=${encodeURIComponent(status)}` : "";
  return get<{ tasks?: TaskQueueItem[]; total?: number }>(`/api/tasks/list${qs}`);
}

export async function deleteTaskFromQueue(id: string): Promise<{ status?: string; id?: string }> {
  return del<{ status?: string; id?: string }>(`/api/tasks/${encodeURIComponent(id)}`);
}

export async function retryTask(id: string): Promise<{ status?: string; id?: string }> {
  return post<{ status?: string; id?: string }>(`/api/tasks/${encodeURIComponent(id)}/retry`, {});
}

export async function cleanupSessions(): Promise<{ sessions_deleted?: number }> {
  return post<{ sessions_deleted?: number }>("/api/sessions/cleanup", {});
}

export async function listAuditRecent(limit = 200): Promise<AuditRecentResponse> {
  const n = Number.isFinite(limit) ? Math.max(1, Math.min(1000, Math.floor(limit))) : 200;
  return get<AuditRecentResponse>(`/api/audit/recent?n=${encodeURIComponent(String(n))}`);
}

export async function verifyAuditChain(): Promise<AuditVerifyResponse> {
  return get<AuditVerifyResponse>("/api/audit/verify");
}

export async function listApprovals(): Promise<ApprovalItem[]> {
  const data = await get<{ approvals?: ApprovalItem[]; total?: number }>("/api/approvals");
  return data.approvals ?? [];
}

export async function approveApproval(id: string, totpCode?: string): Promise<ApiActionResponse> {
  const body = totpCode ? { totp_code: totpCode } : {};
  return post<ApiActionResponse>(`/api/approvals/${encodeURIComponent(id)}/approve`, body);
}

// ── TOTP second-factor management ──

export interface TotpSetupResponse {
  otpauth_uri: string;
  secret: string;
  qr_code: string | null;
  recovery_codes: string[];
  message: string;
}

export interface TotpStatusResponse {
  enrolled: boolean;
  confirmed: boolean;
  enforced: boolean;
  remaining_recovery_codes: number;
}

export async function totpSetup(currentCode?: string): Promise<TotpSetupResponse> {
  const body = currentCode ? { current_code: currentCode } : {};
  return post<TotpSetupResponse>("/api/approvals/totp/setup", body);
}

export async function totpConfirm(code: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/approvals/totp/confirm", { code });
}

export async function totpStatus(): Promise<TotpStatusResponse> {
  return get<TotpStatusResponse>("/api/approvals/totp/status");
}

export async function totpRevoke(code: string): Promise<ApiActionResponse> {
  const response = await fetch("/api/approvals/totp/revoke", {
    method: "POST",
    headers: buildHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ code }),
  });
  if (!response.ok) throw await parseError(response);
  return response.json();
}

export async function rejectApproval(id: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/approvals/${encodeURIComponent(id)}/reject`, {});
}

/**
 * List only pending approval requests, optionally filtered by agent ID.
 */
export async function listPendingApprovals(agentId?: string): Promise<ApprovalItem[]> {
  const all = await listApprovals();
  return all.filter(
    (a) => a.status === "pending" && (!agentId || a.agent_id === agentId),
  );
}

/**
 * Resolve a pending approval request (approve or deny).
 */
export async function resolveApproval(id: string, approved: boolean): Promise<void> {
  if (approved) {
    await approveApproval(id);
  } else {
    await rejectApproval(id);
  }
}

export async function fetchApprovalCount(): Promise<number> {
  const data = await get<{ pending: number }>("/api/approvals/count");
  return data.pending ?? 0;
}

export async function batchResolveApprovals(
  ids: string[],
  decision: "approve" | "reject"
): Promise<{ results: Array<{ id: string; status: string; message?: string }> }> {
  return post("/api/approvals/batch", { ids, decision });
}

export async function modifyAndRetryApproval(
  id: string,
  feedback: string
): Promise<{ id: string; status: string; decided_at: string }> {
  return post(`/api/approvals/${encodeURIComponent(id)}/modify`, { feedback });
}

export interface ApprovalAuditEntry {
  id: string;
  request_id: string;
  agent_id: string;
  tool_name: string;
  description: string;
  action_summary: string;
  risk_level: string;
  decision: string;
  decided_by?: string;
  decided_at: string;
  requested_at: string;
  feedback?: string;
}

export async function queryApprovalAudit(params: {
  limit?: number;
  offset?: number;
  agent_id?: string;
  tool_name?: string;
}): Promise<{ entries: ApprovalAuditEntry[]; total: number }> {
  const query = new URLSearchParams();
  if (params.limit != null) query.set("limit", String(params.limit));
  if (params.offset != null) query.set("offset", String(params.offset));
  if (params.agent_id) query.set("agent_id", params.agent_id);
  if (params.tool_name) query.set("tool_name", params.tool_name);
  return get(`/api/approvals/audit?${query.toString()}`);
}

export async function switchAgentSession(
  agentId: string,
  sessionId: string
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(
    `/api/agents/${encodeURIComponent(agentId)}/sessions/${encodeURIComponent(sessionId)}/switch`,
    {}
  );
}

export async function listAgentSessions(agentId: string): Promise<SessionListItem[]> {
  const data = await get<{ sessions?: SessionListItem[] }>(
    `/api/agents/${encodeURIComponent(agentId)}/sessions`
  );
  return data.sessions ?? [];
}

export async function createAgentSession(
  agentId: string,
  label?: string
): Promise<{ session_id: string; agent_id: string; label?: string }> {
  return post(`/api/agents/${encodeURIComponent(agentId)}/sessions`, label ? { label } : {});
}

export async function listSessions(): Promise<SessionListItem[]> {
  // Bumped past the server's default page size (50) so list-row aggregates
  // (sessions/cost in the agent row) don't silently clip when an agent's
  // sessions aren't in the latest 50 globally. Modern backends embed
  // `sessions_24h` / `cost_24h` directly on each AgentItem (see
  // `enrich_agent_json`), so this scan is now only the *fallback* path
  // for older daemons; the detail-panel KPI tile reads from
  // `GET /api/agents/{id}/stats` and never touches this list.
  // TODO: drop the fallback (and this whole call from AgentsPage) once
  // the minimum supported daemon version is past the embed change.
  const data = await get<{ sessions?: SessionListItem[] }>("/api/sessions?limit=500");
  return data.sessions ?? [];
}

export async function getSessionDetails(sessionId: string): Promise<SessionDetailResponse> {
  return get<SessionDetailResponse>(`/api/sessions/${encodeURIComponent(sessionId)}`);
}

export async function deleteSession(sessionId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/sessions/${encodeURIComponent(sessionId)}`);
}

export async function setSessionLabel(
  sessionId: string,
  label: string | null
): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/sessions/${encodeURIComponent(sessionId)}/label`, {
    label
  });
}

export async function listMemories(params?: {
  agentId?: string;
  offset?: number;
  limit?: number;
  category?: string;
}): Promise<MemoryListResponse> {
  const offset = Number.isFinite(params?.offset) ? Math.max(0, Math.floor(params?.offset ?? 0)) : 0;
  const limit = Number.isFinite(params?.limit) ? Math.max(1, Math.floor(params?.limit ?? 20)) : 20;
  const query = new URLSearchParams();
  query.set("offset", String(offset));
  query.set("limit", String(limit));
  if (params?.category) query.set("category", params.category);

  const path = params?.agentId
    ? `/api/memory/agents/${encodeURIComponent(params.agentId)}?${query.toString()}`
    : `/api/memory?${query.toString()}`;
  return get<MemoryListResponse>(path);
}

export async function searchMemories(params: {
  query: string;
  agentId?: string;
  limit?: number;
}): Promise<MemoryItem[]> {
  const limit = Number.isFinite(params.limit) ? Math.max(1, Math.floor(params.limit ?? 20)) : 20;
  const query = new URLSearchParams();
  query.set("q", params.query);
  query.set("limit", String(limit));

  const path = params.agentId
    ? `/api/memory/agents/${encodeURIComponent(params.agentId)}/search?${query.toString()}`
    : `/api/memory/search?${query.toString()}`;
  const data = await get<{ memories?: MemoryItem[] }>(path);
  return data.memories ?? [];
}

export async function getMemoryStats(agentId?: string): Promise<MemoryStatsResponse> {
  if (agentId) {
    return get<MemoryStatsResponse>(`/api/memory/agents/${encodeURIComponent(agentId)}/stats`);
  }
  return get<MemoryStatsResponse>("/api/memory/stats");
}

// List the per-agent KV memory store (always available — independent of
// `[proactive_memory] enabled`). Used as the fallback view when proactive
// memory is disabled, and as a complementary view when it is enabled.
export async function getAgentKvMemory(agentId: string): Promise<AgentKvResponse> {
  return get<AgentKvResponse>(`/api/memory/agents/${encodeURIComponent(agentId)}/kv`);
}

export async function addMemoryFromText(
  content: string,
  options: { level?: string; agentId?: string } = {}
): Promise<ApiActionResponse> {
  const { level, agentId } = options;
  return post<ApiActionResponse>("/api/memory", {
    messages: [{ role: "user", content }],
    ...(level ? { level } : {}),
    ...(agentId ? { agent_id: agentId } : {})
  });
}

export async function updateMemory(memoryId: string, content: string): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/memory/items/${encodeURIComponent(memoryId)}`, {
    content
  });
}

export async function deleteMemory(memoryId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/memory/items/${encodeURIComponent(memoryId)}`);
}

export async function cleanupMemories(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/memory/cleanup", {});
}

export async function decayMemories(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/memory/decay", {});
}

export async function listUsageByAgent(): Promise<UsageByAgentItem[]> {
  const data = await get<{ agents?: UsageByAgentItem[] }>("/api/usage");
  return data.agents ?? [];
}

export async function getUsageSummary(): Promise<UsageSummaryResponse> {
  return get<UsageSummaryResponse>("/api/usage/summary");
}

export async function listUsageByModel(): Promise<UsageByModelItem[]> {
  const data = await get<{ models?: UsageByModelItem[] }>("/api/usage/by-model");
  return data.models ?? [];
}

export async function getUsageByModelPerformance(): Promise<ModelPerformanceItem[]> {
  const data = await get<{ models?: ModelPerformanceItem[] }>("/api/usage/by-model/performance");
  return data.models ?? [];
}

export async function getUsageDaily(): Promise<UsageDailyResponse> {
  return get<UsageDailyResponse>("/api/usage/daily");
}

export interface BudgetStatus {
  max_hourly_usd?: number;
  max_daily_usd?: number;
  max_monthly_usd?: number;
  alert_threshold?: number;
  default_max_llm_tokens_per_hour?: number;
  [key: string]: unknown;
}

export async function getBudgetStatus(): Promise<BudgetStatus> {
  return get<BudgetStatus>("/api/budget");
}

export async function updateBudget(payload: Partial<BudgetStatus>): Promise<ApiActionResponse> {
  return put<ApiActionResponse>("/api/budget", payload);
}

export async function suspendAgent(agentId: string): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/suspend`, {});
}

export async function resumeAgent(agentId: string): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/agents/${encodeURIComponent(agentId)}/resume`, {});
}

export async function spawnAgent(req: {
  manifest_toml?: string;
  template?: string;
  name?: string;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/agents", req);
}

export async function getCommsTopology(): Promise<CommsTopology> {
  return get<CommsTopology>("/api/comms/topology");
}

export async function listCommsEvents(limit = 200): Promise<CommsEventItem[]> {
  const n = Number.isFinite(limit) ? Math.max(1, Math.min(500, Math.floor(limit))) : 200;
  return get<CommsEventItem[]>(`/api/comms/events?limit=${encodeURIComponent(String(n))}`);
}

export async function sendCommsMessage(payload: {
  from_agent_id: string;
  to_agent_id: string;
  message: string;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/comms/send", payload);
}

export async function postCommsTask(payload: {
  title: string;
  description?: string;
  assigned_to?: string;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/comms/task", payload);
}

export async function listHands(): Promise<HandDefinitionItem[]> {
  const data = await get<{ hands?: HandDefinitionItem[]; total?: number }>("/api/hands");
  return data.hands ?? [];
}

export async function getHandManifestToml(handId: string): Promise<string> {
  return getText(`/api/hands/${encodeURIComponent(handId)}/manifest`);
}

export async function getRawConfigToml(): Promise<string> {
  return getText("/api/config/export");
}

export async function listActiveHands(): Promise<HandInstanceItem[]> {
  const data = await get<{ instances?: HandInstanceItem[]; total?: number }>("/api/hands/active");
  return data.instances ?? [];
}

export async function activateHand(
  handId: string,
  config?: Record<string, unknown>
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/hands/${encodeURIComponent(handId)}/activate`, {
    config: config ?? {}
  });
}

export async function pauseHand(instanceId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/hands/instances/${encodeURIComponent(instanceId)}/pause`, {});
}

export async function resumeHand(instanceId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/hands/instances/${encodeURIComponent(instanceId)}/resume`, {});
}

export async function deactivateHand(instanceId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/hands/instances/${encodeURIComponent(instanceId)}`);
}

/** Uninstall a user-installed hand. Fails with 404 for built-ins and
 *  409 if there is still a live instance. Callers should deactivate
 *  first, then call this. */
export async function uninstallHand(handId: string): Promise<{ status: string; hand_id: string }> {
  return del(`/api/hands/${encodeURIComponent(handId)}`);
}

export async function getHandStats(instanceId: string): Promise<HandStatsResponse> {
  return get<HandStatsResponse>(`/api/hands/instances/${encodeURIComponent(instanceId)}/stats`);
}

export interface HandSettingOptionStatus {
  value?: string;
  label?: string;
  provider_env?: string | null;
  binary?: string | null;
  available?: boolean;
}

export interface HandSettingStatus {
  key?: string;
  label?: string;
  description?: string;
  setting_type?: string;
  default?: string;
  options?: HandSettingOptionStatus[];
}

export interface HandSettingsResponse {
  hand_id?: string;
  settings?: HandSettingStatus[];
  current_values?: Record<string, unknown>;
}

export async function getHandDetail(handId: string): Promise<HandDefinitionItem> {
  return get<HandDefinitionItem>(`/api/hands/${encodeURIComponent(handId)}`);
}

export async function getHandSettings(handId: string): Promise<HandSettingsResponse> {
  return get<HandSettingsResponse>(`/api/hands/${encodeURIComponent(handId)}/settings`);
}

export async function setHandSecret(handId: string, key: string, value: string): Promise<{ ok: boolean }> {
  return post<{ ok: boolean }>(`/api/hands/${encodeURIComponent(handId)}/secret`, { key, value });
}

/** Update mutable settings on an active hand instance. The backend returns
 *  404 if no instance exists for the hand — callers should guard accordingly. */
export async function updateHandSettings(
  handId: string,
  config: Record<string, unknown>,
): Promise<{ status: string; hand_id: string; instance_id: string; config: Record<string, unknown> }> {
  return put(`/api/hands/${encodeURIComponent(handId)}/settings`, config);
}

export interface HandMessageResponse {
  response: string;
  input_tokens?: number;
  output_tokens?: number;
  iterations?: number;
  cost_usd?: number;
}

export type SessionBlock =
  | { type: "text"; text: string }
  | { type: "tool_use"; id: string; name: string; input: unknown }
  | { type: "tool_result"; tool_use_id: string; name: string; content: string; is_error: boolean };

export interface HandSessionMessage {
  role: string;
  content: string;
  timestamp?: string;
  blocks?: SessionBlock[];
}

export async function sendHandMessage(instanceId: string, message: string): Promise<HandMessageResponse> {
  return post<HandMessageResponse>(
    `/api/hands/instances/${encodeURIComponent(instanceId)}/message`,
    { message },
    LONG_RUNNING_TIMEOUT_MS
  );
}

export async function getHandSession(instanceId: string): Promise<{ messages: HandSessionMessage[] }> {
  return get<{ messages: HandSessionMessage[] }>(`/api/hands/instances/${encodeURIComponent(instanceId)}/session`);
}

export interface HandInstanceStatus {
  instance_id: string;
  hand_id: string;
  hand_name?: string;
  hand_icon?: string;
  status: string;
  activated_at: string;
  config: Record<string, unknown>;
  agent?: {
    id: string;
    name: string;
    state: string;
    model: { provider: string; model: string };
    iterations_total?: number;
    session_id: string;
  };
}

export async function getHandInstanceStatus(instanceId: string): Promise<HandInstanceStatus> {
  return get<HandInstanceStatus>(`/api/hands/instances/${encodeURIComponent(instanceId)}/status`);
}

export async function listGoals(): Promise<GoalItem[]> {
  const data = await get<{ goals?: GoalItem[]; total?: number }>("/api/goals");
  return data.goals ?? [];
}

export interface GoalTemplate {
  id: string;
  name: string;
  icon: string;
  description: string;
  goals: { title: string; description: string; status: string }[];
}

export async function listGoalTemplates(): Promise<GoalTemplate[]> {
  const data = await get<{ templates?: GoalTemplate[] }>("/api/goals/templates");
  return data.templates ?? [];
}

export async function createGoal(payload: {
  title: string;
  description?: string;
  parent_id?: string;
  agent_id?: string;
  status?: string;
  progress?: number;
}): Promise<GoalItem> {
  return post<GoalItem>("/api/goals", payload);
}

export async function updateGoal(
  goalId: string,
  payload: {
    title?: string;
    description?: string;
    status?: string;
    progress?: number;
    parent_id?: string | null;
    agent_id?: string | null;
  }
): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/goals/${encodeURIComponent(goalId)}`, payload);
}

export async function deleteGoal(goalId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/goals/${encodeURIComponent(goalId)}`);
}

// ── Network / Peers ──────────────────────────────────

export interface NetworkStatusResponse {
  online?: boolean;
  node_id?: string;
  protocol_version?: string;
  listen_addr?: string;
  peer_count?: number;
  [key: string]: unknown;
}

export interface PeerItem {
  id: string;
  addr?: string;
  name?: string;
  status?: string;
  connected_at?: string;
  last_seen?: string;
  version?: string;
  [key: string]: unknown;
}

export async function getNetworkStatus(): Promise<NetworkStatusResponse> {
  return get<NetworkStatusResponse>("/api/network/status");
}

export async function listPeers(): Promise<PeerItem[]> {
  const data = await get<{ peers?: PeerItem[] }>("/api/peers");
  return data.peers ?? [];
}

export async function getPeerDetail(peerId: string): Promise<PeerItem> {
  return get<PeerItem>(`/api/peers/${encodeURIComponent(peerId)}`);
}

// ── A2A (Agent-to-Agent) ─────────────────────────────

export interface A2AAgentItem {
  url?: string;
  name?: string;
  description?: string;
  version?: string;
  skills?: string[];
  status?: string;
  discovered_at?: string;
  [key: string]: unknown;
}

export interface A2ATaskStatus {
  id?: string;
  status?: string;
  result?: string;
  error?: string;
  created_at?: string;
  completed_at?: string;
  [key: string]: unknown;
}

export async function listA2AAgents(): Promise<A2AAgentItem[]> {
  const data = await get<{ agents?: A2AAgentItem[] }>("/api/a2a/agents");
  return data.agents ?? [];
}

export async function discoverA2AAgent(url: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/a2a/discover", { url });
}

export async function sendA2ATask(payload: {
  agent_url: string;
  message: string;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/a2a/send", payload);
}

export async function getA2ATaskStatus(taskId: string): Promise<A2ATaskStatus> {
  return get<A2ATaskStatus>(`/api/a2a/tasks/${encodeURIComponent(taskId)}/status`);
}

export function setApiKey(key: string) {
  // #3620: Store in sessionStorage (tab-scoped, reduced XSS exposure) and
  // remove any stale localStorage copy so the two stores don't diverge.
  sessionStorage.setItem("librefang-api-key", key);
  localStorage.removeItem("librefang-api-key");
  // Reset the 401-fired guard so future unauthorized responses
  // (e.g. after token expiry) can re-trigger the login dialog.
  _unauthorizedFired = false;
}

export function clearApiKey() {
  sessionStorage.removeItem("librefang-api-key");
  localStorage.removeItem("librefang-api-key");
}

/** Invalidate the server-side session + cookie, then clear the local token.
 *  Safe to call even when the token is already gone. */
export async function dashboardLogout(): Promise<void> {
  try {
    await fetch("/api/auth/logout", {
      method: "POST",
      credentials: "same-origin",
      headers: authHeader(),
    });
  } catch {
    // Network failure shouldn't block local cleanup — fall through.
  }
  clearApiKey();
}

export function hasApiKey(): boolean {
  const key = getStoredApiKey();
  return !!key && key.length > 0;
}

export type AuthMode = "credentials" | "api_key" | "hybrid" | "none";

export async function checkDashboardAuthMode(): Promise<AuthMode> {
  try {
    const resp = await fetch("/api/auth/dashboard-check");
    if (!resp.ok) return "none";
    const data = await resp.json();
    return (data.mode as AuthMode) || "none";
  } catch {
    return "none";
  }
}

export async function getDashboardUsername(): Promise<string> {
  try {
    const resp = await fetch("/api/auth/dashboard-check");
    if (!resp.ok) return "";
    const data = await resp.json();
    return (data.username as string) || "";
  } catch {
    return "";
  }
}

export async function dashboardLogin(username: string, password: string, totpCode?: string): Promise<{ ok: boolean; token?: string; error?: string; requires_totp?: boolean }> {
  try {
    const body: Record<string, string> = { username, password };
    if (totpCode) body.totp_code = totpCode;
    const resp = await fetch("/api/auth/dashboard-login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    const data = await resp.json();
    if (data.ok && data.token) {
      setApiKey(data.token);
    }
    return data;
  } catch (e: unknown) {
    return { ok: false, error: e instanceof Error ? e.message : "Network error" };
  }
}

export async function verifyStoredAuth(): Promise<boolean> {
  if (!hasApiKey()) {
    return false;
  }

  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const response = await fetch("/api/security", {
        headers: buildHeaders(),
      });
      if (response.status === 401) {
        clearApiKey();
        return false;
      }
      return response.ok;
    } catch {
      await new Promise((r) => setTimeout(r, 1000));
    }
  }

  return false;
}

export async function getMetricsText(): Promise<string> {
  return getText("/api/metrics");
}

// ── Plugins ──────────────────────────────────────────

export interface PluginItem {
  name: string;
  version: string;
  description?: string;
  author?: string;
  hooks_valid: boolean;
  size_bytes: number;
  path?: string;
  hooks?: { ingest?: boolean; after_turn?: boolean };
}

export interface RegistryPluginListing {
  name: string;
  installed: boolean;
  version?: string | null;
  description?: string | null;
  author?: string | null;
  hooks?: string[];
}

export interface RegistryEntry {
  name: string;
  github_repo: string;
  error?: string | null;
  plugins: RegistryPluginListing[];
}

export async function listPlugins(): Promise<{ plugins: PluginItem[]; total: number; plugins_dir: string }> {
  return get<{ plugins: PluginItem[]; total: number; plugins_dir: string }>("/api/plugins");
}

export async function getPlugin(name: string): Promise<PluginItem> {
  return get<PluginItem>(`/api/plugins/${encodeURIComponent(name)}`);
}

export async function installPlugin(source: { source: string; name?: string; path?: string; url?: string; branch?: string; github_repo?: string }): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/install", source, LONG_RUNNING_TIMEOUT_MS);
}

export async function uninstallPlugin(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/uninstall", { name });
}

export async function scaffoldPlugin(
  name: string,
  description: string,
  runtime?: string,
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/scaffold", { name, description, runtime });
}

export async function installPluginDeps(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(
    `/api/plugins/${encodeURIComponent(name)}/install-deps`,
    {},
    LONG_RUNNING_TIMEOUT_MS
  );
}

export async function listPluginRegistries(): Promise<{ registries: RegistryEntry[] }> {
  return get<{ registries: RegistryEntry[] }>("/api/plugins/registries");
}

export interface PromptVersion {
  id: string;
  agent_id: string;
  version: number;
  content_hash: string;
  system_prompt: string;
  tools: string[];
  variables: string[];
  created_at: string;
  created_by: string;
  is_active: boolean;
  description?: string;
}

export interface PromptExperiment {
  id: string;
  name: string;
  agent_id: string;
  status: "draft" | "running" | "paused" | "completed";
  traffic_split: number[];
  success_criteria: {
    require_user_helpful: boolean;
    require_no_tool_errors: boolean;
    require_non_empty: boolean;
    custom_min_score?: number;
  };
  started_at?: string;
  ended_at?: string;
  created_at: string;
  variants: ExperimentVariant[];
}

export interface ExperimentVariant {
  id?: string;
  name: string;
  prompt_version_id: string;
  description?: string;
}

export interface ExperimentVariantMetrics {
  variant_id: string;
  variant_name: string;
  total_requests: number;
  successful_requests: number;
  failed_requests: number;
  success_rate: number;
  avg_latency_ms: number;
  avg_cost_usd: number;
  total_cost_usd: number;
}

export async function listPromptVersions(agentId: string): Promise<PromptVersion[]> {
  return get<PromptVersion[]>(`/api/agents/${encodeURIComponent(agentId)}/prompts/versions`);
}

export async function createPromptVersion(agentId: string, version: Omit<PromptVersion, "id" | "agent_id" | "created_at" | "is_active">): Promise<PromptVersion> {
  return post<PromptVersion>(`/api/agents/${encodeURIComponent(agentId)}/prompts/versions`, version);
}

export async function deletePromptVersion(versionId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/prompts/versions/${encodeURIComponent(versionId)}`);
}

export async function activatePromptVersion(versionId: string, agentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/prompts/versions/${encodeURIComponent(versionId)}/activate`, { agent_id: agentId });
}

export async function listExperiments(agentId: string): Promise<PromptExperiment[]> {
  return get<PromptExperiment[]>(`/api/agents/${encodeURIComponent(agentId)}/prompts/experiments`);
}

export async function createExperiment(agentId: string, experiment: Omit<PromptExperiment, "id" | "agent_id" | "created_at">): Promise<PromptExperiment> {
  return post<PromptExperiment>(`/api/agents/${encodeURIComponent(agentId)}/prompts/experiments`, experiment);
}

export async function startExperiment(experimentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/prompts/experiments/${encodeURIComponent(experimentId)}/start`, {});
}

export async function pauseExperiment(experimentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/prompts/experiments/${encodeURIComponent(experimentId)}/pause`, {});
}

export async function completeExperiment(experimentId: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/prompts/experiments/${encodeURIComponent(experimentId)}/complete`, {});
}

export async function getExperimentMetrics(experimentId: string): Promise<ExperimentVariantMetrics[]> {
  return get<ExperimentVariantMetrics[]>(`/api/prompts/experiments/${encodeURIComponent(experimentId)}/metrics`);
}

// ---------------------------------------------------------------------------
// Registry Schema
// ---------------------------------------------------------------------------

export interface RegistrySchemaField {
  type: string;
  required?: boolean;
  description?: string;
  example?: unknown;
  default?: unknown;
  options?: string[];
  item_type?: string;
}

export interface RegistrySchemaSection {
  description?: string;
  repeatable?: boolean;
  fields?: Record<string, RegistrySchemaField>;
  sections?: Record<string, RegistrySchemaSection>;
}

export interface RegistrySchema {
  description?: string;
  file_pattern?: string;
  fields?: Record<string, RegistrySchemaField>;
  sections?: Record<string, RegistrySchemaSection>;
}

export async function fetchAllRegistrySchemas(): Promise<Record<string, RegistrySchema>> {
  return get<Record<string, RegistrySchema>>("/api/registry/schema");
}

export async function fetchRegistrySchema(contentType: string): Promise<RegistrySchema> {
  return get<RegistrySchema>(`/api/registry/schema/${encodeURIComponent(contentType)}`);
}

export interface CreateRegistryContentResponse {
  ok: boolean;
  content_type: string;
  identifier: string;
  path: string;
}

export async function createRegistryContent(
  contentType: string,
  values: Record<string, unknown>,
): Promise<CreateRegistryContentResponse> {
  return post<CreateRegistryContentResponse>(
    `/api/registry/content/${encodeURIComponent(contentType)}`,
    values,
  );
}

// ---------------------------------------------------------------------------
// Auth — change password
// ---------------------------------------------------------------------------

// ── MCP Servers API ─────────────────────────────────────────────────────
//
// The MCP API is unified under `/api/mcp/*` — both raw-configured servers
// (from `config.toml`) and catalog-installed servers live in the same
// `/api/mcp/servers` collection. `template_id` tracks provenance when a
// server was installed from a catalog entry.

export interface McpTransport {
  type: "stdio" | "sse" | "http";
  command?: string;
  args?: string[];
  url?: string;
}

/** TaintRuleId — must match the snake-cased serde tag of the Rust enum. */
export type TaintRuleId =
  | "authorization_literal"
  | "key_value_secret"
  | "well_known_prefix"
  | "opaque_token"
  | "pii_email"
  | "pii_phone"
  | "pii_credit_card"
  | "pii_ssn"
  | "sensitive_key_name";

/** Tool-level baseline action when no path entry matches. */
export type McpTaintToolAction = "scan" | "skip";

/** Severity action for a named rule set. */
export type McpTaintRuleSetAction = "block" | "warn" | "log";

export interface McpTaintPathPolicy {
  skip_rules: TaintRuleId[];
}

export interface McpTaintToolPolicy {
  default?: McpTaintToolAction;
  paths?: Record<string, McpTaintPathPolicy>;
  rule_sets?: string[];
}

export interface McpTaintPolicy {
  tools?: Record<string, McpTaintToolPolicy>;
}

export interface NamedTaintRuleSet {
  name: string;
  action?: McpTaintRuleSetAction;
  rules?: TaintRuleId[];
}

export interface McpServerConfigured {
  /** Stable identifier; falls back to `name` when the backend omits it. */
  id?: string;
  name: string;
  transport: McpTransport;
  timeout_secs?: number;
  env?: string[];
  headers?: string[];
  /** Catalog template this server was installed from, when applicable. */
  template_id?: string;
  auth_state?: { state: string; auth_url?: string; message?: string };
  /** Issue #3050: per-server taint scanning toggle. */
  taint_scanning?: boolean;
  /** Issue #3050: granular per-tool / per-path / per-rule taint policy. */
  taint_policy?: McpTaintPolicy;
}

export interface McpServerConnected {
  name: string;
  tools_count: number;
  tools: { name: string; description?: string }[];
  connected: boolean;
}

export interface McpServersResponse {
  configured: McpServerConfigured[];
  connected: McpServerConnected[];
  total_configured: number;
  total_connected: number;
}

export async function listMcpServers(): Promise<McpServersResponse> {
  return get<McpServersResponse>("/api/mcp/servers");
}

export async function getMcpServer(id: string): Promise<McpServerConfigured> {
  return get<McpServerConfigured>(`/api/mcp/servers/${encodeURIComponent(id)}`);
}

// ── MCP Catalog (read-only browse of registry templates) ────────

export interface McpCatalogRequiredEnv {
  name: string;
  label: string;
  help?: string;
  is_secret?: boolean;
  get_url?: string;
}

export interface McpCatalogEntry {
  id: string;
  name: string;
  description: string;
  icon?: string;
  category?: string;
  installed: boolean;
  tags?: string[];
  transport?: McpTransport;
  required_env?: McpCatalogRequiredEnv[];
  has_oauth?: boolean;
  setup_instructions?: string;
}

export interface McpCatalogResponse {
  entries: McpCatalogEntry[];
  count: number;
}

export async function listMcpCatalog(): Promise<McpCatalogResponse> {
  return get<McpCatalogResponse>("/api/mcp/catalog");
}

export async function getMcpCatalogEntry(id: string): Promise<McpCatalogEntry> {
  return get<McpCatalogEntry>(`/api/mcp/catalog/${encodeURIComponent(id)}`);
}

// ── MCP Server mutations ────────────────────────────────────────────────

/** Install a server from a catalog template with the supplied credentials. */
export type AddMcpServerFromTemplate = {
  template_id: string;
  credentials?: Record<string, string>;
};

/** Create a server from a raw spec (same shape as a configured entry). */
export type AddMcpServerSpec = Omit<McpServerConfigured, "id"> & { name: string };

/**
 * Body is either `{ template_id, credentials }` to install a catalog entry,
 * or a raw `McpServerConfigured` spec. The backend disambiguates by the
 * presence of `template_id`.
 */
export async function addMcpServer(
  body: AddMcpServerFromTemplate | AddMcpServerSpec,
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/mcp/servers", body);
}

export async function updateMcpServer(
  id: string,
  server: Partial<McpServerConfigured>,
): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/mcp/servers/${encodeURIComponent(id)}`, server);
}

export interface PatchMcpTaintRequest {
  taint_scanning?: boolean;
  taint_policy?: McpTaintPolicy;
}

export async function patchMcpServerTaint(
  id: string,
  body: PatchMcpTaintRequest,
): Promise<ApiActionResponse> {
  return patch<ApiActionResponse>(
    `/api/mcp/servers/${encodeURIComponent(id)}/taint`,
    body,
  );
}

export async function deleteMcpServer(id: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/mcp/servers/${encodeURIComponent(id)}`);
}

export async function reconnectMcpServer(id: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/mcp/servers/${encodeURIComponent(id)}/reconnect`, {});
}

// ── MCP Health & Reload ────────────────────────────────────────────────

export interface McpHealthEntry {
  id: string;
  status: string;
  tool_count?: number;
  last_ok?: string | null;
  last_error?: string | null;
  consecutive_failures?: number;
  reconnecting?: boolean;
  reconnect_attempts?: number;
  connected_since?: string | null;
}

export interface McpHealthResponse {
  health: McpHealthEntry[];
  count: number;
}

export async function getMcpHealth(): Promise<McpHealthResponse> {
  return get<McpHealthResponse>("/api/mcp/health");
}

export async function reloadMcp(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/mcp/reload", {});
}

// ── MCP `[[taint_rules]]` Registry ──────────────────────────────────────

/** Summary of one named taint rule set defined by `[[taint_rules]]`. */
export interface McpTaintRuleSummary {
  /** Identifier referenced by `McpTaintToolPolicy.rule_sets`. */
  name: string;
  /** Severity action this set applies when one of its rules fires. */
  action: "block" | "warn" | "log";
  /** Number of `TaintRuleId` variants this set covers (display-only). */
  rule_count: number;
}

/**
 * Read-only list of `[[taint_rules]]` for dashboard validation.
 *
 * Used by `TaintPolicyEditor` to flag rule_set names that don't match any
 * registered set — without this, typos sit silent in production until a
 * scanner WARN line happens to be noticed in logs.
 */
export async function listMcpTaintRules(): Promise<McpTaintRuleSummary[]> {
  return get<McpTaintRuleSummary[]>("/api/mcp/taint-rules");
}

// ── MCP OAuth Auth ──────────────────────────────────────────────────────

export interface McpAuthStatusResponse {
  server: string;
  auth: { state: string; auth_url?: string; message?: string };
}

export interface McpAuthStartResponse {
  auth_url: string;
  server: string;
}

export async function getMcpAuthStatus(id: string): Promise<McpAuthStatusResponse> {
  return get<McpAuthStatusResponse>(`/api/mcp/servers/${encodeURIComponent(id)}/auth/status`);
}

export async function startMcpAuth(id: string): Promise<McpAuthStartResponse> {
  return post<McpAuthStartResponse>(`/api/mcp/servers/${encodeURIComponent(id)}/auth/start`, {});
}

export async function revokeMcpAuth(id: string): Promise<{ server: string; state: string }> {
  return del<{ server: string; state: string }>(`/api/mcp/servers/${encodeURIComponent(id)}/auth/revoke`);
}

// ---------------------------------------------------------------------------

export async function changePassword(
  currentPassword: string,
  newPassword: string | null,
  newUsername: string | null,
): Promise<{ ok: boolean; error?: string; message?: string }> {
  return post<{ ok: boolean; error?: string; message?: string }>(
    "/api/auth/change-password",
    {
      current_password: currentPassword,
      ...(newPassword ? { new_password: newPassword } : {}),
      ...(newUsername ? { new_username: newUsername } : {}),
    },
  );
}

// ---------------------------------------------------------------------------
// Terminal (tmux windows)
// ---------------------------------------------------------------------------

export interface TerminalWindow {
  id: string;
  index: number;
  name: string;
  active: boolean;
}

export interface TerminalHealth {
  ok: boolean;
  tmux: boolean;
  max_windows: number;
  os: string;
}

export async function getTerminalHealth(): Promise<TerminalHealth> {
  return get<TerminalHealth>("/api/terminal/health");
}

export async function listTerminalWindows(): Promise<TerminalWindow[]> {
  const response = await fetch("/api/terminal/windows", {
    headers: buildHeaders(),
  });
  if (!response.ok) throw await parseError(response);
  const data = (await response.json()) as { windows?: TerminalWindow[] } | TerminalWindow[];
  return Array.isArray(data) ? data : (data.windows ?? []);
}

export async function createTerminalWindow(body: { name?: string } = {}): Promise<void> {
  const response = await fetch("/api/terminal/windows", {
    method: "POST",
    headers: buildHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify(body),
  });
  if (!response.ok) throw await parseError(response);
}

export async function renameTerminalWindow(windowId: string, name: string): Promise<void> {
  const response = await fetch(
    `/api/terminal/windows/${encodeURIComponent(windowId)}`,
    {
      method: "PATCH",
      headers: buildHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({ name }),
    },
  );
  if (!response.ok) throw await parseError(response);
}

export async function deleteTerminalWindow(windowId: string): Promise<void> {
  const response = await fetch(
    `/api/terminal/windows/${encodeURIComponent(windowId)}`,
    { method: "DELETE", headers: buildHeaders() },
  );
  if (!response.ok) throw await parseError(response);
}

// ── Auto-Dream (background memory consolidation) ──────────────────────

export type AutoDreamStatusName =
  | "running"
  | "completed"
  | "failed"
  | "aborted";

export interface AutoDreamTurn {
  text: string;
  tool_use_count: number;
}

/** Token accounting snapshot from a completed dream. Populated only on
 * the `completed` status; absent for running / failed / aborted. The
 * cache_* fields let the dashboard surface cache-hit rate so operators
 * can see the forkedAgent cost savings in real terms. */
export interface AutoDreamUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_input_tokens: number;
  cache_creation_input_tokens: number;
  iterations: number;
  latency_ms: number;
  cost_usd?: number;
}

export interface AutoDreamProgress {
  task_id: string;
  agent_id: string;
  started_at_ms: number;
  ended_at_ms: number | null;
  status: AutoDreamStatusName;
  phase: string;
  tool_use_count: number;
  memories_touched: string[];
  turns: AutoDreamTurn[];
  error: string | null;
  usage?: AutoDreamUsage;
}

export interface AutoDreamAgentStatus {
  agent_id: string;
  agent_name: string;
  auto_dream_enabled: boolean;
  last_consolidated_at_ms: number;
  /** Unix-ms when the time gate reopens. Omitted when the agent has never been dreamed. */
  next_eligible_at_ms?: number;
  /** Hours since last consolidation. Omitted when the agent has never been dreamed. */
  hours_since_last?: number;
  sessions_since_last: number;
  /** Resolved min_hours (manifest override or global default). */
  effective_min_hours: number;
  /** Resolved min_sessions (manifest override or global default; 0 = gate disabled). */
  effective_min_sessions: number;
  lock_path: string;
  progress: AutoDreamProgress | null;
  can_abort: boolean;
}

export interface AutoDreamStatus {
  enabled: boolean;
  min_hours: number;
  min_sessions: number;
  check_interval_secs: number;
  lock_dir: string;
  agents: AutoDreamAgentStatus[];
}

export interface AutoDreamTriggerOutcome {
  fired: boolean;
  agent_id: string;
  task_id: string | null;
  reason: string;
}

export interface AutoDreamAbortOutcome {
  aborted: boolean;
  agent_id: string;
  reason: string;
}

export async function getAutoDreamStatus(): Promise<AutoDreamStatus> {
  return get<AutoDreamStatus>("/api/auto-dream/status");
}

export async function triggerAutoDream(agentId: string): Promise<AutoDreamTriggerOutcome> {
  return post<AutoDreamTriggerOutcome>(
    `/api/auto-dream/agents/${encodeURIComponent(agentId)}/trigger`,
    {},
  );
}

export async function abortAutoDream(agentId: string): Promise<AutoDreamAbortOutcome> {
  return post<AutoDreamAbortOutcome>(
    `/api/auto-dream/agents/${encodeURIComponent(agentId)}/abort`,
    {},
  );
}

export async function setAutoDreamEnabled(
  agentId: string,
  enabled: boolean,
): Promise<{ agent_id: string; enabled: boolean }> {
  return put<{ agent_id: string; enabled: boolean }>(
    `/api/auto-dream/agents/${encodeURIComponent(agentId)}/enabled`,
    { enabled },
  );
}

// ---------------------------------------------------------------------------
// RBAC users (Phase 4 / M6)
// ---------------------------------------------------------------------------

export type UserRoleName = "owner" | "admin" | "user" | "viewer";

export interface UserItem {
  name: string;
  role: string;
  channel_bindings: Record<string, string>;
  has_api_key: boolean;
  // Summary flags — true when the user overrides the role default for
  // that slot. Bodies stay on the per-user detail endpoints.
  has_policy: boolean;
  has_memory_access: boolean;
  has_budget: boolean;
}

export interface UserUpsertPayload {
  name: string;
  role: string;
  channel_bindings?: Record<string, string>;
  api_key_hash?: string | null;
}

export interface BulkImportRow {
  index: number;
  name: string;
  status: string;
  error: string | null;
}

export interface BulkImportResult {
  created: number;
  updated: number;
  failed: number;
  dry_run: boolean;
  rows: BulkImportRow[];
}

export async function listUsers(): Promise<UserItem[]> {
  return get<UserItem[]>("/api/users");
}

export async function getUser(name: string): Promise<UserItem> {
  return get<UserItem>(`/api/users/${encodeURIComponent(name)}`);
}

export async function createUser(payload: UserUpsertPayload): Promise<UserItem> {
  return post<UserItem>("/api/users", payload);
}

export async function updateUser(
  originalName: string,
  payload: UserUpsertPayload,
): Promise<UserItem> {
  return put<UserItem>(
    `/api/users/${encodeURIComponent(originalName)}`,
    payload,
  );
}

export async function deleteUser(name: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/users/${encodeURIComponent(name)}`);
}

export async function importUsers(
  rows: UserUpsertPayload[],
  options: { dryRun?: boolean } = {},
): Promise<BulkImportResult> {
  return post<BulkImportResult>("/api/users/import", {
    rows,
    dry_run: options.dryRun ?? false,
  });
}

// ---------------------------------------------------------------------------
// API-key rotation (RBAC follow-up to #3054 / M3 / M6)
//
// Owner-only. Returns the new plaintext key in the response — that is the
// only time the server exposes it; we never log, persist, or re-derive it.
// ---------------------------------------------------------------------------

export interface RotateUserKeyResponse {
  status: string;
  new_api_key: string;
  sessions_invalidated: number;
}

export async function rotateUserKey(name: string): Promise<RotateUserKeyResponse> {
  return post<RotateUserKeyResponse>(
    `/api/users/${encodeURIComponent(name)}/rotate-key`,
    {},
  );
}

// ---------------------------------------------------------------------------
// Audit query (RBAC M5 / #3203). Shape mirrors `routes/audit.rs::audit_query`
// — keep field names in lockstep, the server returns raw `serde_json::Value`
// so a drift here is silently wrong wire-format on the page.
// ---------------------------------------------------------------------------

export interface AuditQueryFilters {
  user?: string; // UUID or configured name
  action?: string; // AuditAction variant name, case-insensitive
  agent?: string;
  channel?: string;
  from?: string; // ISO-8601 lower bound (inclusive)
  to?: string; // ISO-8601 upper bound (inclusive)
  limit?: number; // default 200, hard cap 5000
}

export interface AuditQueryEntry {
  seq: number;
  timestamp: string;
  agent_id: string;
  action: string;
  detail: string;
  outcome: string;
  user_id: string | null;
  channel: string | null;
  hash: string;
}

export interface AuditQueryResponse {
  entries: AuditQueryEntry[];
  count: number;
  limit: number;
}

export async function queryAudit(
  filters: AuditQueryFilters = {},
): Promise<AuditQueryResponse> {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(filters)) {
    if (v === undefined || v === null || v === "") continue;
    params.set(k, String(v));
  }
  const qs = params.toString();
  return get<AuditQueryResponse>(
    `/api/audit/query${qs ? `?${qs}` : ""}`,
  );
}

// ---------------------------------------------------------------------------
// Per-user budget (RBAC M5)
// ---------------------------------------------------------------------------

/// Per-window spend + cap pair returned by GET /api/budget/users/{user_id}.
export interface UserBudgetWindow {
  spend: number;
  limit: number;
  pct: number;
}

/// Shape returned by GET /api/budget/users/{user_id} — see
/// `routes/budget.rs::user_budget_detail`.
export interface UserBudgetResponse {
  user_id: string;
  name: string | null;
  role: string | null;
  hourly: UserBudgetWindow;
  daily: UserBudgetWindow;
  monthly: UserBudgetWindow;
  alert_threshold: number;
  alert_breach: boolean;
  /// True once the M5 enforcement arm is wired (commit 4a00a646). Kept
  /// in the payload so the dashboard can surface a "deferred" notice
  /// against older daemons that may still report `false`.
  enforced: boolean;
}

/// Body shape for PUT /api/budget/users/{user_id}. Mirrors
/// `librefang_types::config::UserBudgetConfig`. Any window left at 0
/// means "unlimited on that window"; same semantics as the kernel
/// metering check.
export interface UserBudgetPayload {
  max_hourly_usd: number;
  max_daily_usd: number;
  max_monthly_usd: number;
  alert_threshold: number;
}

export async function getUserBudget(name: string): Promise<UserBudgetResponse> {
  return get<UserBudgetResponse>(
    `/api/budget/users/${encodeURIComponent(name)}`,
  );
}

export async function updateUserBudget(
  name: string,
  payload: UserBudgetPayload,
): Promise<{ status: string; budget: UserBudgetPayload }> {
  return put(
    `/api/budget/users/${encodeURIComponent(name)}`,
    payload,
  );
}

export async function deleteUserBudget(
  name: string,
): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(
    `/api/budget/users/${encodeURIComponent(name)}`,
  );
}

// ---------------------------------------------------------------------------
// Per-user permission policy (RBAC M3 / #3205 — wired to the real daemon)
// ---------------------------------------------------------------------------

export interface UserToolPolicy {
  allowed_tools: string[];
  denied_tools: string[];
}

export interface UserToolCategories {
  allowed_groups: string[];
  denied_groups: string[];
}

export interface UserMemoryAccess {
  readable_namespaces: string[];
  writable_namespaces: string[];
  pii_access: boolean;
  export_allowed: boolean;
  delete_allowed: boolean;
}

export interface ChannelToolPolicy {
  allowed_tools: string[];
  denied_tools: string[];
}

// Mirrors the `UserPolicyView` returned by `GET /api/users/{name}/policy`.
// `null` on a top-level slot = "no opinion configured" (kernel falls back
// to role-default). Empty `channel_tool_rules` map = no per-channel rules.
export interface PermissionPolicy {
  tool_policy: UserToolPolicy | null;
  tool_categories: UserToolCategories | null;
  memory_access: UserMemoryAccess | null;
  channel_tool_rules: Record<string, ChannelToolPolicy>;
}

// PUT body shape: every key independently nullable. `undefined` = preserve
// existing, `null` = clear. `channel_tool_rules` collapses absent/null to
// "preserve"; pass `{}` to clear.
export interface PermissionPolicyUpdate {
  tool_policy?: UserToolPolicy | null;
  tool_categories?: UserToolCategories | null;
  memory_access?: UserMemoryAccess | null;
  channel_tool_rules?: Record<string, ChannelToolPolicy>;
}

export async function getUserPolicy(name: string): Promise<PermissionPolicy> {
  return get<PermissionPolicy>(
    `/api/users/${encodeURIComponent(name)}/policy`,
  );
}

export async function updateUserPolicy(
  name: string,
  policy: PermissionPolicyUpdate,
): Promise<PermissionPolicy> {
  return put<PermissionPolicy>(
    `/api/users/${encodeURIComponent(name)}/policy`,
    policy,
  );
}

// ---------------------------------------------------------------------------
// Effective permissions snapshot (RBAC follow-up to M3/M5/M6)
// ---------------------------------------------------------------------------

// Mirrors the shape of `librefang_kernel::auth::EffectivePermissions`.
// Per-slice fields are nullable so the simulator can distinguish "no policy
// declared" (null) from "explicit empty allow-list" (object with empty
// arrays). Server returns 404 for unknown users — callers handle that via
// the query hook's error state, not by getting a synthesised default.

export interface EffectiveToolPolicy {
  allowed_tools: string[];
  denied_tools: string[];
}

export interface EffectiveToolCategories {
  allowed_groups: string[];
  denied_groups: string[];
}

export interface EffectiveMemoryAccess {
  readable_namespaces: string[];
  writable_namespaces: string[];
  pii_access: boolean;
  export_allowed: boolean;
  delete_allowed: boolean;
}

export interface EffectiveBudget {
  max_hourly_usd: number;
  max_daily_usd: number;
  max_monthly_usd: number;
  alert_threshold: number;
}

export interface EffectiveChannelToolPolicy {
  allowed_tools: string[];
  denied_tools: string[];
}

export interface EffectivePermissions {
  user_id: string;
  name: string;
  role: string;
  tool_policy: EffectiveToolPolicy | null;
  tool_categories: EffectiveToolCategories | null;
  memory_access: EffectiveMemoryAccess | null;
  budget: EffectiveBudget | null;
  channel_tool_rules: Record<string, EffectiveChannelToolPolicy>;
  channel_bindings: Record<string, string>;
}

export async function getEffectivePermissions(
  name: string,
): Promise<EffectivePermissions> {
  return get<EffectivePermissions>(
    `/api/authz/effective/${encodeURIComponent(name)}`,
  );
}

// ---------------------------------------------------------------------------
// Device pairing
// ---------------------------------------------------------------------------

export interface PairingRequestResult {
  token: string;
  qr_uri: string;
  expires_at: string;
}

export interface PairedDevice {
  device_id: string;
  display_name: string;
  platform: string;
  paired_at: string;
}

// Pairing completion is initiated by the mobile client against an arbitrary
// daemon URL (cross-origin), so it lives in `lib/mutations/connection.ts`
// rather than this same-origin api module.

export async function createPairingRequest(): Promise<PairingRequestResult> {
  return post<PairingRequestResult>("/api/pairing/request", {});
}

export async function listPairedDevices(): Promise<PairedDevice[]> {
  return get<PairedDevice[]>("/api/pairing/devices");
}

export async function removePairedDevice(deviceId: string): Promise<void> {
  return del<void>(`/api/pairing/devices/${encodeURIComponent(deviceId)}`);
}
