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

export interface WorkflowItem {
  id: string;
  name?: string;
  description?: string;
  steps?: number;
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

function authHeader(): HeadersInit {
  const token = localStorage.getItem("librefang-api-key") || "";
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function parseError(response: Response): Promise<Error> {
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

async function post<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "POST",
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

export async function listChannels(): Promise<ChannelItem[]> {
  const data = await get<ChannelsResponse>("/api/channels");
  return data.channels ?? [];
}

export async function testChannel(channelName: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/channels/${encodeURIComponent(channelName)}/test`, {});
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
}): Promise<ApiActionResponse> {
  return post<ApiActionResponse>("/api/workflows", payload);
}

export async function runWorkflow(workflowId: string, input: string): Promise<ApiActionResponse> {
  return post<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}/run`, {
    input
  });
}

export async function deleteWorkflow(workflowId: string): Promise<ApiActionResponse> {
  return del<ApiActionResponse>(`/api/workflows/${encodeURIComponent(workflowId)}`);
}

export async function listWorkflowRuns(workflowId: string): Promise<WorkflowRunItem[]> {
  return get<WorkflowRunItem[]>(`/api/workflows/${encodeURIComponent(workflowId)}/runs`);
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
  return get<TriggerItem[]>("/api/triggers");
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
