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
  // memory
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
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
  CronJobItem,
  HandDefinitionItem,
  HandInstanceItem,
  HandSessionMessage,
  HandSettingsResponse,
  HandStatsResponse,
  McpAuthStartResponse,
  McpAuthStatusResponse,
  MemoryItem,
  ModelOverrides,
  MediaImageResult,
  MediaMusicResult,
  MediaProvider,
  MediaVideoStatus,
  SpeechResult,
  TerminalHealth,
  TerminalWindow,
} from "../../api";
