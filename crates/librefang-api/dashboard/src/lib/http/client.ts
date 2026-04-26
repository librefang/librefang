// Typed HTTP client surface for the data layer.
//
// This file is the *only* entry point that `src/lib/queries/*` and
// `src/lib/mutations/*` should use to reach `src/api.ts`. Re-exports are
// explicit (no `export *`) so:
//   - the surface consumed by hooks is a documented, reviewable whitelist
//     rather than whatever `api.ts` happens to export today;
//   - removing or renaming a symbol in `api.ts` breaks here first, not in
//     every hook file;
//   - `ApiError` (thrown by `api.ts::parseError`) is re-exported alongside
//     the functions so hooks can narrow on `err instanceof ApiError`
//     without reaching into `../http/errors` directly.

export { ApiError } from "./errors";

// ---------------------------------------------------------------------------
// Query functions (read)
// ---------------------------------------------------------------------------
export {
  // agents
  listAgents,
  getAgentDetail,
  listAgentSessions,
  listAgentTemplates,
  listPromptVersions,
  listExperiments,
  getExperimentMetrics,
  // analytics / usage / budget
  getUsageSummary,
  listUsageByAgent,
  listUsageByModel,
  getUsageDaily,
  getUsageByModelPerformance,
  getBudgetStatus,
  // channels & comms
  listChannels,
  getCommsTopology,
  listCommsEvents,
  // config & registry
  getFullConfig,
  getConfigSchema,
  fetchRegistrySchema,
  getRawConfigToml,
  // goals
  listGoals,
  listGoalTemplates,
  // hands
  listHands,
  listActiveHands,
  getHandDetail,
  getHandSettings,
  getHandStats,
  getHandSession,
  getHandInstanceStatus,
  getHandManifestToml,
  getMetricsText,
  // mcp
  listMcpServers,
  getMcpServer,
  listMcpCatalog,
  getMcpCatalogEntry,
  getMcpHealth,
  getMcpAuthStatus,
  listMcpTaintRules,
  // memory
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
  getAgentKvMemory,
  // models
  listModels,
  getModelOverrides,
  // providers
  listProviders,
  // network / peers / a2a
  getNetworkStatus,
  listPeers,
  listA2AAgents,
  getA2ATaskStatus,
  // plugins
  listPlugins,
  listPluginRegistries,
  // schedules & triggers
  listSchedules,
  listTriggers,
  getTrigger,
  // sessions
  listSessions,
  getSessionDetails,
  // skills (local + hubs)
  listSkills,
  getSkillDetail,
  getSupportingFile,
  clawhubBrowse,
  clawhubSearch,
  clawhubGetSkill,
  clawhubCnBrowse,
  clawhubCnSearch,
  clawhubCnGetSkill,
  skillhubBrowse,
  skillhubSearch,
  skillhubGetSkill,
  fanghubListSkills,
  listMediaProviders,
  pollVideo,
  // workflows
  listWorkflows,
  listWorkflowRuns,
  getWorkflowRun,
  listWorkflowTemplates,
  // terminal
  getTerminalHealth,
  listTerminalWindows,
  // auto-dream
  getAutoDreamStatus,
  // overview
  loadDashboardSnapshot,
  getVersionInfo,
  // runtime
  getStatus,
  getQueueStatus,
  getHealthDetail,
  getSecurityStatus,
  listBackups,
  getTaskQueueStatus,
  listTaskQueue,
  listCronJobs,
  // audit
  listAuditRecent,
  verifyAuditChain,
  queryAudit,
  // users (RBAC M6)
  listUsers,
  getUser,
  // per-user budget (M5) / policy (M3 #3205 — wired)
  getUserBudget,
  getUserPolicy,
  // effective permissions snapshot (RBAC follow-up — backs the simulator)
  getEffectivePermissions,
} from "../../api";

export type {
  UserBudgetResponse,
  UserBudgetWindow,
  UserBudgetPayload,
} from "../../api";

// ---------------------------------------------------------------------------
// Mutation functions (write)
// ---------------------------------------------------------------------------
export {
  // agents
  spawnAgent,
  cloneAgent,
  stopAgent,
  suspendAgent,
  resumeAgent,
  deleteAgent,
  clearAgentHistory,
  patchAgent,
  patchAgentConfig,
  patchHandAgentRuntimeConfig,
  clearHandAgentRuntimeConfig,
  createAgentSession,
  switchAgentSession,
  deleteSession,
  setSessionLabel,
  deletePromptVersion,
  activatePromptVersion,
  createPromptVersion,
  createExperiment,
  startExperiment,
  pauseExperiment,
  completeExperiment,
  // approvals
  resolveApproval,
  // analytics
  updateBudget,
  // channels & comms
  configureChannel,
  testChannel,
  reloadChannels,
  sendCommsMessage,
  postCommsTask,
  // attachments
  uploadAgentFile,
  // media
  generateImage,
  synthesizeSpeech,
  submitVideo,
  generateMusic,
  // config
  setConfigValue,
  reloadConfig,
  // goals
  createGoal,
  updateGoal,
  deleteGoal,
  // hands
  activateHand,
  deactivateHand,
  pauseHand,
  resumeHand,
  uninstallHand,
  setHandSecret,
  updateHandSettings,
  sendHandMessage,
  // mcp
  addMcpServer,
  updateMcpServer,
  patchMcpServerTaint,
  deleteMcpServer,
  reconnectMcpServer,
  reloadMcp,
  startMcpAuth,
  revokeMcpAuth,
  // memory
  addMemoryFromText,
  updateMemory,
  deleteMemory,
  cleanupMemories,
  updateMemoryConfig,
  // models
  addCustomModel,
  removeCustomModel,
  updateModelOverrides,
  deleteModelOverrides,
  // providers
  testProvider,
  setProviderKey,
  deleteProviderKey,
  setProviderUrl,
  setDefaultProvider,
  // network / a2a
  discoverA2AAgent,
  sendA2ATask,
  // plugins
  installPlugin,
  uninstallPlugin,
  scaffoldPlugin,
  installPluginDeps,
  // schedules & triggers
  createSchedule,
  updateSchedule,
  deleteSchedule,
  runSchedule,
  createTrigger,
  updateTrigger,
  deleteTrigger,
  // skills
  createSkill,
  reloadSkills,
  evolveUpdateSkill,
  evolvePatchSkill,
  evolveRollbackSkill,
  evolveDeleteSkill,
  evolveWriteFile,
  evolveRemoveFile,
  installSkill,
  uninstallSkill,
  clawhubInstall,
  clawhubCnInstall,
  skillhubInstall,
  // workflows
  runWorkflow,
  dryRunWorkflow,
  deleteWorkflow,
  createWorkflow,
  updateWorkflow,
  instantiateTemplate,
  saveWorkflowAsTemplate,
  // terminal
  createTerminalWindow,
  renameTerminalWindow,
  deleteTerminalWindow,
  // auto-dream
  triggerAutoDream,
  abortAutoDream,
  setAutoDreamEnabled,
  // users (RBAC M6)
  createUser,
  updateUser,
  deleteUser,
  importUsers,
  rotateUserKey,
  // per-user policy (M3 #3205)
  updateUserPolicy,
  // per-user budget (RBAC M5)
  updateUserBudget,
  deleteUserBudget,
} from "../../api";

// ---------------------------------------------------------------------------
// Type re-exports used by hooks and pages
// ---------------------------------------------------------------------------
export type {
  A2AAgentItem,
  A2ATaskStatus,
  AutoDreamAbortOutcome,
  AutoDreamAgentStatus,
  AutoDreamProgress,
  AutoDreamStatus,
  AutoDreamStatusName,
  AutoDreamTriggerOutcome,
  AutoDreamTurn,
  CronDeliveryTarget,
  CronDeliveryTargetType,
  CronJobItem,
  HandDefinitionItem,
  HandInstanceItem,
  HandSessionMessage,
  HandSettingsResponse,
  HandStatsResponse,
  McpAuthStartResponse,
  McpAuthStatusResponse,
  MemoryItem,
  AgentKvPair,
  AgentKvResponse,
  ModelOverrides,
  MediaImageResult,
  MediaMusicResult,
  MediaProvider,
  MediaVideoStatus,
  MediaVideoSubmitResult,
  SpeechResult,
  TerminalHealth,
  TerminalWindow,
  // users / RBAC
  UserItem,
  UserUpsertPayload,
  UserRoleName,
  BulkImportRow,
  BulkImportResult,
  RotateUserKeyResponse,
  // audit / per-user budget / policy
  AuditQueryEntry,
  AuditQueryFilters,
  AuditQueryResponse,
  UserBudgetEntry,
  UserBudgetResponse,
  PermissionPolicy,
  PermissionPolicyUpdate,
  UserToolPolicy,
  UserToolCategories,
  UserMemoryAccess,
  ChannelToolPolicy,
  // effective permissions snapshot (RBAC follow-up)
  EffectivePermissions,
  EffectiveToolPolicy,
  EffectiveToolCategories,
  EffectiveMemoryAccess,
  EffectiveBudget,
  EffectiveChannelToolPolicy,
} from "../../api";
