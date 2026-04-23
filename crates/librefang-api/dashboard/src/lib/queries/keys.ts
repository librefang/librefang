// Query key factories — hierarchical pattern for precise invalidation.
//
// Convention:
//   all          → broadest prefix (for invalidateQueries({ queryKey: xxxKeys.all }))
//   lists()      → all list queries
//   list(filters) → specific list query
//   details()    → all detail queries
//   detail(id)   → specific detail query
//
// All arrays use `as const` for structural stability.

export const autoDreamKeys = {
  all: ["autoDream"] as const,
  status: () => [...autoDreamKeys.all, "status"] as const,
};

export const agentKeys = {
  all: ["agents"] as const,
  lists: () => [...agentKeys.all, "list"] as const,
  list: (opts: { includeHands?: boolean } = {}) =>
    [...agentKeys.lists(), opts] as const,
  details: () => [...agentKeys.all, "detail"] as const,
  detail: (id: string) => [...agentKeys.details(), id] as const,
  templates: () => [...agentKeys.all, "templates"] as const,
  sessions: (agentId: string) =>
    [...agentKeys.all, "sessions", agentId] as const,
  promptVersions: (agentId: string) =>
    [...agentKeys.all, "promptVersions", agentId] as const,
  experiments: (agentId: string) =>
    [...agentKeys.all, "experiments", agentId] as const,
  experimentMetrics: (experimentId: string) =>
    [...agentKeys.all, "experimentMetrics", experimentId] as const,
};

export const modelKeys = {
  all: ["models"] as const,
  lists: () => [...modelKeys.all, "list"] as const,
  list: (filters: {
    provider?: string;
    tier?: string;
    available?: boolean;
  } = {}) => [...modelKeys.lists(), filters] as const,
  details: () => [...modelKeys.all, "detail"] as const,
  detail: (id: string) => [...modelKeys.details(), id] as const,
  overrides: (modelKey: string) =>
    [...modelKeys.all, "overrides", modelKey] as const,
};

export const providerKeys = {
  all: ["providers"] as const,
  lists: () => [...providerKeys.all, "list"] as const,
};

export const channelKeys = {
  all: ["channels"] as const,
  lists: () => [...channelKeys.all, "list"] as const,
};

export const commsKeys = {
  all: ["comms"] as const,
  topology: () => [...commsKeys.all, "topology"] as const,
  events: (limit = 200) => [...commsKeys.all, "events", limit] as const,
};

export const skillKeys = {
  all: ["skills"] as const,
  lists: () => [...skillKeys.all, "list"] as const,
  details: () => [...skillKeys.all, "detail"] as const,
  detail: (name: string) => [...skillKeys.details(), name] as const,
  supportingFiles: (name: string) =>
    [...skillKeys.detail(name), "supportingFile"] as const,
  supportingFile: (name: string, path: string) =>
    [...skillKeys.supportingFiles(name), path] as const,
};

export const clawhubKeys = {
  all: ["clawhub"] as const,
  browse: (filters: {
    sort?: string;
    limit?: number;
    cursor?: string;
  } = {}) => [...clawhubKeys.all, "browse", filters] as const,
  search: (query: string) =>
    [...clawhubKeys.all, "search", query] as const,
  details: () => [...clawhubKeys.all, "detail"] as const,
  detail: (slug: string) => [...clawhubKeys.details(), slug] as const,
};

export const clawhubCnKeys = {
  all: ["clawhub-cn"] as const,
  browse: (filters: {
    sort?: string;
    limit?: number;
    cursor?: string;
  } = {}) => [...clawhubCnKeys.all, "browse", filters] as const,
  search: (query: string) =>
    [...clawhubCnKeys.all, "search", query] as const,
  details: () => [...clawhubCnKeys.all, "detail"] as const,
  detail: (slug: string) => [...clawhubCnKeys.details(), slug] as const,
};

export const skillhubKeys = {
  all: ["skillhub"] as const,
  browse: (sort?: string) => [...skillhubKeys.all, "browse", sort] as const,
  search: (query: string) =>
    [...skillhubKeys.all, "search", query] as const,
  details: () => [...skillhubKeys.all, "detail"] as const,
  detail: (slug: string) => [...skillhubKeys.details(), slug] as const,
};

export const fanghubKeys = {
  all: ["fanghub"] as const,
  lists: () => [...fanghubKeys.all, "list"] as const,
};

export const handKeys = {
  all: ["hands"] as const,
  lists: () => [...handKeys.all, "list"] as const,
  active: () => [...handKeys.all, "active"] as const,
  details: () => [...handKeys.all, "detail"] as const,
  detail: (id: string) => [...handKeys.details(), id] as const,
  settings: (handId: string) =>
    [...handKeys.all, "settings", handId] as const,
  stats: (instanceId: string) =>
    [...handKeys.all, "stats", instanceId] as const,
  statsBatch: (instanceIds: readonly string[]) =>
    [...handKeys.all, "statsBatch", instanceIds] as const,
  session: (instanceId: string) =>
    [...handKeys.all, "session", instanceId] as const,
  instanceStatus: (instanceId: string) =>
    [...handKeys.all, "instanceStatus", instanceId] as const,
  manifest: (handId: string) =>
    [...handKeys.all, "manifest", handId] as const,
};

export const workflowKeys = {
  all: ["workflows"] as const,
  lists: () => [...workflowKeys.all, "list"] as const,
  details: () => [...workflowKeys.all, "detail"] as const,
  detail: (id: string) => [...workflowKeys.details(), id] as const,
  runs: (workflowId: string) =>
    [...workflowKeys.all, "runs", workflowId] as const,
  runDetails: () => [...workflowKeys.all, "runDetail"] as const,
  runDetail: (runId: string) =>
    [...workflowKeys.runDetails(), runId] as const,
  templates: (filters: { q?: string; category?: string } = {}) =>
    [...workflowKeys.all, "templates", filters] as const,
};

export const scheduleKeys = {
  all: ["schedules"] as const,
  lists: () => [...scheduleKeys.all, "list"] as const,
};

export const triggerKeys = {
  all: ["triggers"] as const,
  lists: () => [...triggerKeys.all, "list"] as const,
  list: (agentId?: string) => [...triggerKeys.lists(), agentId] as const,
  details: () => [...triggerKeys.all, "detail"] as const,
  detail: (id: string) => [...triggerKeys.details(), id] as const,
};

export const cronKeys = {
  all: ["cron"] as const,
  jobs: (agentId?: string) =>
    [...cronKeys.all, "jobs", agentId] as const,
};

export const approvalKeys = {
  all: ["approvals"] as const,
  lists: () => [...approvalKeys.all, "list"] as const,
  count: () => [...approvalKeys.all, "count"] as const,
  pending: (agentId?: string | null) =>
    [...approvalKeys.all, "pending", agentId] as const,
  audit: (filters: {
    limit?: number;
    offset?: number;
    agent_id?: string;
    tool_name?: string;
  } = {}) => [...approvalKeys.all, "audit", filters] as const,
};

export const totpKeys = {
  all: ["totp"] as const,
  status: () => [...totpKeys.all, "status"] as const,
};

export const memoryKeys = {
  all: ["memory"] as const,
  lists: () => [...memoryKeys.all, "list"] as const,
  list: (filters: {
    agentId?: string;
    offset?: number;
    limit?: number;
    category?: string;
  } = {}) => [...memoryKeys.lists(), filters] as const,
  searchOrList: (search: string) => [...memoryKeys.lists(), "searchOrList", search] as const,
  statsAll: () => [...memoryKeys.all, "stats"] as const,
  stats: (agentId?: string) =>
    [...memoryKeys.statsAll(), agentId] as const,
  config: () => [...memoryKeys.all, "config"] as const,
};

export const usageKeys = {
  all: ["usage"] as const,
  summary: () => [...usageKeys.all, "summary"] as const,
  byAgent: () => [...usageKeys.all, "byAgent"] as const,
  byModel: () => [...usageKeys.all, "byModel"] as const,
  modelPerformance: () =>
    [...usageKeys.all, "modelPerformance"] as const,
  daily: () => [...usageKeys.all, "daily"] as const,
};

export const budgetKeys = {
  all: ["budget"] as const,
  status: () => [...budgetKeys.all, "status"] as const,
};

export const goalKeys = {
  all: ["goals"] as const,
  lists: () => [...goalKeys.all, "list"] as const,
  templates: () => [...goalKeys.all, "templates"] as const,
};

export const networkKeys = {
  all: ["network"] as const,
  status: () => [...networkKeys.all, "status"] as const,
};

export const peerKeys = {
  all: ["peers"] as const,
  lists: () => [...peerKeys.all, "list"] as const,
  details: () => [...peerKeys.all, "detail"] as const,
  detail: (id: string) => [...peerKeys.details(), id] as const,
};

export const a2aKeys = {
  all: ["a2a"] as const,
  agents: () => [...a2aKeys.all, "agents"] as const,
};

export const sessionKeys = {
  all: ["sessions"] as const,
  lists: () => [...sessionKeys.all, "list"] as const,
  details: () => [...sessionKeys.all, "detail"] as const,
  detail: (id: string) => [...sessionKeys.details(), id] as const,
};

export const overviewKeys = {
  all: ["dashboard"] as const,
  snapshot: () => [...overviewKeys.all, "snapshot"] as const,
  version: () => [...overviewKeys.all, "version"] as const,
};

export const runtimeKeys = {
  all: ["runtime"] as const,
  status: () => [...runtimeKeys.all, "status"] as const,
  queueStatus: () => [...runtimeKeys.all, "queue", "status"] as const,
  healthDetail: () => [...runtimeKeys.all, "health", "detail"] as const,
  security: () => [...runtimeKeys.all, "security"] as const,
  backups: () => [...runtimeKeys.all, "backups"] as const,
  tasks: () => [...runtimeKeys.all, "tasks"] as const,
  taskStatus: () => [...runtimeKeys.tasks(), "status"] as const,
  taskList: (status?: string) =>
    [...runtimeKeys.tasks(), "list", status] as const,
};

export const auditKeys = {
  all: ["audit"] as const,
  recent: (limit: number) => [...auditKeys.all, "recent", limit] as const,
  verify: () => [...auditKeys.all, "verify"] as const,
};

export const mediaKeys = {
  all: ["media"] as const,
  providers: () => [...mediaKeys.all, "providers"] as const,
  videoTasks: () => [...mediaKeys.all, "videoTasks"] as const,
  videoTask: (taskId: string, provider: string) =>
    [...mediaKeys.videoTasks(), taskId, provider] as const,
  // Stable key for the disabled state of useVideoTask — paired with skipToken
  // so every not-yet-submitted render shares the same (unused) cache slot.
  // Shape mirrors `videoTask(taskId, provider)` (4 segments) so both branches
  // of the query are type-compatible under useQuery's generic inference.
  videoTaskDisabled: () =>
    [...mediaKeys.videoTasks(), "__disabled__", "__disabled__"] as const,
};

export const mcpKeys = {
  all: ["mcp"] as const,
  servers: () => [...mcpKeys.all, "servers"] as const,
  server: (id: string) => [...mcpKeys.servers(), id] as const,
  authStatus: (id: string) => [...mcpKeys.all, "authStatus", id] as const,
  catalog: () => [...mcpKeys.all, "catalog"] as const,
  catalogEntry: (id: string) => [...mcpKeys.catalog(), id] as const,
  health: () => [...mcpKeys.all, "health"] as const,
};

export const pluginKeys = {
  all: ["plugins"] as const,
  lists: () => [...pluginKeys.all, "list"] as const,
  registries: () => [...pluginKeys.all, "registries"] as const,
};

export const configKeys = {
  all: ["config"] as const,
  full: () => [...configKeys.all, "full"] as const,
  schema: () => [...configKeys.all, "schema"] as const,
  rawToml: () => [...configKeys.all, "rawToml"] as const,
};

export const registryKeys = {
  all: ["registry"] as const,
  schema: (contentType: string) =>
    [...registryKeys.all, "schema", contentType] as const,
};

export const telemetryKeys = {
  all: ["telemetry"] as const,
  metrics: () => [...telemetryKeys.all, "metrics"] as const,
};

export const terminalKeys = {
  all: ["terminal"] as const,
  health: () => [...terminalKeys.all, "health"] as const,
  windows: () => [...terminalKeys.all, "windows"] as const,
};
