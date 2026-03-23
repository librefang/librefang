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
  network_enabled?: boolean;
  session_count?: number;
}

export interface VersionResponse {
  name?: string;
  version?: string;
  build_date?: string;
  git_sha?: string;
  rust_version?: string;
  platform?: string;
  arch?: string;
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
  key_required?: boolean;
  health?: string;
  media_capabilities?: string[];
}

export interface MediaProvider {
  id: string;
  display_name: string;
  capabilities: string[];
  configured: boolean;
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

export interface MediaVideoStatus {
  state: string;
  error?: string;
}

export interface MediaMusicResult {
  format: string;
  provider: string;
  model: string;
  duration_ms?: number;
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
  value?: unknown;
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
  ready?: boolean;
  profile?: string;
  identity?: AgentIdentity;
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
}

export interface AgentSessionMessage {
  role?: string;
  content?: unknown;
  tools?: AgentTool[];
  images?: AgentSessionImage[];
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
  prompt: string;
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
}

export interface WorkflowRunItem {
  id?: string;
  workflow_name?: string;
  state?: unknown;
  steps_completed?: number;
  started_at?: string;
  completed_at?: string | null;
}

export interface ScheduleItem {
  id: string;
  name?: string;
  cron?: string;
  description?: string;
  message?: string;
  enabled?: boolean;
  created_at?: string;
  last_run?: string | null;
  run_count?: number;
  agent_id?: string;
  agent?: string;
  schedule_input?: string;
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

export interface QueueStatusResponse {
  lanes?: QueueLaneStatus[];
  config?: {
    max_depth_per_agent?: number;
    max_depth_global?: number;
    task_ttl_secs?: number;
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
  created_at?: string;
  label?: string | null;
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
}

export interface MemoryListResponse {
  memories?: MemoryItem[];
  total?: number;
  offset?: number;
  limit?: number;
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
}

export interface HandInstanceItem {
  instance_id: string;
  hand_id?: string;
  status?: string;
  agent_id?: string;
  agent_name?: string;
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

// Global 401 handler — set by App.tsx to trigger login screen
let _onUnauthorized: (() => void) | null = null;
let _unauthorizedFired = false;
export function setOnUnauthorized(fn: (() => void) | null) {
  _onUnauthorized = fn;
  _unauthorizedFired = false;
}

function authHeader(): HeadersInit {
  const lang = localStorage.getItem("i18nextLng") || navigator.language || "en";
  const token = localStorage.getItem("librefang-api-key") || "";
  const headers: HeadersInit = { "Accept-Language": lang };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  return headers;
}

async function parseError(response: Response): Promise<Error> {
  // If 401, trigger global logout (only once to prevent infinite loop)
  if (response.status === 401 && _onUnauthorized && !_unauthorizedFired) {
    _unauthorizedFired = true;
    clearApiKey();
    _onUnauthorized();
  }
  const text = await response.text();
  let message = response.statusText;
  try {
    const json = JSON.parse(text) as Json;
    if (typeof json.error === "string") {
      message = json.error;
    }
  } catch {
    // ignore parse errors
  }
  return new Error(message || `HTTP ${response.status}`);
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: {
      "Content-Type": "application/json",
      ...authHeader()
    }
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

async function post<T>(path: string, body: unknown, timeout = 60000): Promise<T> {
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeout);

  try {
    const response = await fetch(path, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...authHeader()
      },
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
    if (error instanceof Error && error.name === "AbortError") {
      throw new Error("Request timeout - installation may take too long");
    }
    throw error;
  }
}

async function put<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "PUT",
    headers: {
      "Content-Type": "application/json",
      ...authHeader()
    },
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
    headers: {
      "Content-Type": "application/json",
      ...authHeader()
    }
  });
  if (!response.ok) {
    throw await parseError(response);
  }
  return (await response.json()) as T;
}

export async function loadDashboardSnapshot(): Promise<DashboardSnapshot> {
  const [health, status, providersRaw, channelsRaw, skillsRaw, agents, workflows] = await Promise.all([
    get<HealthResponse>("/api/health"),
    get<StatusResponse>("/api/status"),
    get<ProvidersResponse>("/api/providers"),
    get<ChannelsResponse>("/api/channels"),
    get<SkillsResponse>("/api/skills"),
    listAgents(),
    get<{ workflows?: any[] }>("/api/workflows")
  ]);

  return {
    health,
    status,
    providers: providersRaw.providers ?? [],
    channels: channelsRaw.channels ?? [],
    agents: agents ?? [],
    skillCount: skillsRaw.skills?.length ?? 0,
    workflowCount: workflows.workflows?.length ?? 0
  };
}


export async function getAgentDetail(agentId: string): Promise<any> {
  return get<any>(`/api/agents/${encodeURIComponent(agentId)}`);
}

export async function listAgents(): Promise<AgentItem[]> {
  const data = await get<PaginatedResponse<AgentItem>>(
    "/api/agents?limit=200&sort=last_active&order=desc"
  );
  return data.items ?? [];
}

export async function loadAgentSession(agentId: string): Promise<AgentSessionResponse> {
  return get<AgentSessionResponse>(`/api/agents/${encodeURIComponent(agentId)}/session`);
}

export async function sendAgentMessage(
  agentId: string,
  message: string
): Promise<AgentMessageResponse> {
  return post<AgentMessageResponse>(`/api/agents/${encodeURIComponent(agentId)}/message`, {
    message
  });
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

export async function setProviderKey(providerId: string, key: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/key`, { key });
}

export async function deleteProviderKey(providerId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/key`);
}

export async function setProviderUrl(providerId: string, baseUrl: string): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/providers/${encodeURIComponent(providerId)}/url`, { base_url: baseUrl });
}

// ── Media generation API ──────────────────────────────────────────────

export async function listMediaProviders(): Promise<MediaProvider[]> {
  const data = await get<{ providers: MediaProvider[] }>("/api/media/providers");
  return data.providers ?? [];
}

export async function generateImage(req: { prompt: string; provider?: string; model?: string; count?: number; aspect_ratio?: string }): Promise<MediaImageResult> {
  return post<MediaImageResult>("/api/media/image", req);
}

export async function synthesizeSpeech(req: { text: string; provider?: string; model?: string; voice?: string; format?: string }): Promise<Blob> {
  const resp = await fetch("/api/media/speech", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(req),
  });
  if (!resp.ok) throw new Error(`TTS failed: ${resp.status}`);
  return resp.blob();
}

export async function submitVideo(req: { prompt: string; provider?: string; model?: string }): Promise<MediaVideoSubmitResult> {
  return post<MediaVideoSubmitResult>("/api/media/video", req);
}

export async function pollVideo(taskId: string): Promise<MediaVideoStatus> {
  return get<MediaVideoStatus>(`/api/media/video/${encodeURIComponent(taskId)}`);
}

export async function generateMusic(req: { prompt?: string; lyrics?: string; provider?: string; model?: string; instrumental?: boolean }): Promise<Blob> {
  const resp = await fetch("/api/media/music", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(req),
  });
  if (!resp.ok) throw new Error(`Music generation failed: ${resp.status}`);
  return resp.blob();
}

export async function listChannels(): Promise<ChannelItem[]> {
  const data = await get<ChannelsResponse>("/api/channels");
  return data.channels ?? [];
}

export async function testChannel(channelName: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/channels/${encodeURIComponent(channelName)}/test`, {});
}

export async function configureChannel(channelName: string, config: Record<string, unknown>): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/channels/${encodeURIComponent(channelName)}/configure`, config);
}

export async function reloadChannels(): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/channels/reload", {});
}

export async function listSkills(): Promise<SkillItem[]> {
  const data = await get<SkillsResponse>("/api/skills");
  return data.skills ?? [];
}

export async function installSkill(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skills/install", { name });
}

export async function uninstallSkill(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skills/uninstall", { name });
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

export async function clawhubInstall(slug: string, version?: string): Promise<ApiActionResponse> {
  // Use default timeout for install - ClawHub can be slow
  return post<ApiActionResponse>("/api/clawhub/install", { slug, version: version || "latest" });
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

export async function skillhubInstall(slug: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/skillhub/install", { slug });
}

export async function skillhubGetSkill(slug: string): Promise<ClawHubSkillDetail> {
  return get<ClawHubSkillDetail>(`/api/skillhub/skill/${encodeURIComponent(slug)}`);
}

// ── Workflow Templates ────────────────────────────────

export interface TemplateParameter {
  name: string;
  description?: string;
  param_type?: string;
  default?: unknown;
  required?: boolean;
}

export interface WorkflowTemplate {
  id: string;
  name: string;
  description?: string;
  category?: string;
  tags?: string[];
  parameters?: TemplateParameter[];
  steps?: WorkflowStep[];
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
  return get<WorkflowItem[]>("/api/workflows");
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
  layout?: any;
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/workflows", payload);
}

export async function getWorkflow(workflowId: string): Promise<any> {
  return get<any>(`/api/workflows/${encodeURIComponent(workflowId)}`);
}

export async function runWorkflow(workflowId: string, input: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}/run`, {
    input
  }, 300000); // 5 min timeout — workflows run multiple LLM steps
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
  layout?: any;
}): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}`, payload);
}

export async function listWorkflowRuns(workflowId: string): Promise<WorkflowRunItem[]> {
  return get<WorkflowRunItem[]>(`/api/workflows/${encodeURIComponent(workflowId)}/runs`);
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
  agent_id: string;
  message: string;
  enabled?: boolean;
}): Promise<ScheduleItem> {
  return post<ScheduleItem>("/api/schedules", payload);
}

export async function updateSchedule(
  scheduleId: string,
  payload: {
    enabled?: boolean;
    name?: string;
    cron?: string;
    agent_id?: string;
    message?: string;
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
  const data = await get<any>("/api/triggers");
  return data.triggers ?? data ?? [];
}

export async function updateTrigger(
  triggerId: string,
  payload: { enabled: boolean }
): Promise<ApiActionResponse> {
  return put<ApiActionResponse>(`/api/triggers/${encodeURIComponent(triggerId)}`, payload);
}

export async function deleteTrigger(triggerId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/triggers/${encodeURIComponent(triggerId)}`);
}

export async function listCronJobs(): Promise<CronJobItem[]> {
  const data = await get<{ jobs?: CronJobItem[]; total?: number }>("/api/cron/jobs");
  return data.jobs ?? [];
}

export async function getVersionInfo(): Promise<VersionResponse> {
  return get<VersionResponse>("/api/version");
}

export async function getQueueStatus(): Promise<QueueStatusResponse> {
  return get<QueueStatusResponse>("/api/queue/status");
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

export async function approveApproval(id: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/approvals/${encodeURIComponent(id)}/approve`, {});
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

export async function switchAgentSession(
  agentId: string,
  sessionId: string
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(
    `/api/agents/${encodeURIComponent(agentId)}/sessions/${encodeURIComponent(sessionId)}/switch`,
    {}
  );
}

export async function listSessions(): Promise<SessionListItem[]> {
  const data = await get<{ sessions?: SessionListItem[] }>("/api/sessions");
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

export async function addMemoryFromText(
  content: string,
  agentId?: string
): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/memory", {
    messages: [{ role: "user", content }],
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

export async function spawnAgent(req: { manifest_toml?: string; template?: string }): Promise<ApiActionResponse> {
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

export interface HandMessageResponse {
  response: string;
  input_tokens?: number;
  output_tokens?: number;
  iterations?: number;
  cost_usd?: number;
}

export interface HandSessionMessage {
  role: string;
  content: string;
  timestamp?: string;
}

export async function sendHandMessage(instanceId: string, message: string): Promise<HandMessageResponse> {
  return post<HandMessageResponse>(`/api/hands/instances/${encodeURIComponent(instanceId)}/message`, { message });
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

// ── Auth check ───────────────────────────────────────

export async function checkAuthRequired(): Promise<boolean> {
  // Retry a few times in case daemon is still booting
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const response = await fetch("/api/status", {
        headers: { ...authHeader() },
      });
      return response.status === 401;
    } catch {
      // Network error — daemon may not be up yet, wait and retry
      await new Promise((r) => setTimeout(r, 1000));
    }
  }
  return false;
}

export function setApiKey(key: string) {
  localStorage.setItem("librefang-api-key", key);
}

export function clearApiKey() {
  localStorage.removeItem("librefang-api-key");
}

export function hasApiKey(): boolean {
  const key = localStorage.getItem("librefang-api-key");
  return !!key && key.length > 0;
}

export type AuthMode = "credentials" | "api_key" | "none";

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

export async function dashboardLogin(username: string, password: string): Promise<{ ok: boolean; token?: string; error?: string }> {
  try {
    const resp = await fetch("/api/auth/dashboard-login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });
    const data = await resp.json();
    if (data.ok && data.token) {
      setApiKey(data.token);
    }
    return data;
  } catch (e: any) {
    return { ok: false, error: e.message || "Network error" };
  }
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

export interface RegistryEntry {
  name: string;
  github_repo: string;
  error?: string | null;
  plugins: Array<{ name: string; installed: boolean }>;
}

export async function listPlugins(): Promise<{ plugins: PluginItem[]; total: number; plugins_dir: string }> {
  return get<{ plugins: PluginItem[]; total: number; plugins_dir: string }>("/api/plugins");
}

export async function getPlugin(name: string): Promise<PluginItem> {
  return get<PluginItem>(`/api/plugins/${encodeURIComponent(name)}`);
}

export async function installPlugin(source: { source: string; name?: string; path?: string; url?: string; branch?: string; github_repo?: string }): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/install", source);
}

export async function uninstallPlugin(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/uninstall", { name });
}

export async function scaffoldPlugin(name: string, description: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/plugins/scaffold", { name, description });
}

export async function installPluginDeps(name: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/plugins/${encodeURIComponent(name)}/install-deps`, {});
}

export async function listPluginRegistries(): Promise<{ registries: RegistryEntry[] }> {
  return get<{ registries: RegistryEntry[] }>("/api/plugins/registries");
}
