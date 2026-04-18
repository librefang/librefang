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
  // mcp
  listMcpServers,
  getMcpServer,
  listMcpCatalog,
  getMcpCatalogEntry,
  getMcpHealth,
  // memory
  listMemories,
  searchMemories,
  getMemoryStats,
  getMemoryConfig,
  // models
  listModels,
  getModelOverrides,
  // network / peers / a2a
  getNetworkStatus,
  listPeers,
  listA2AAgents,
  // plugins
  listPlugins,
  listPluginRegistries,
  // schedules & triggers
  listSchedules,
  listTriggers,
  // sessions
  listSessions,
  getSessionDetails,
  // skills (local + hubs)
  listSkills,
  clawhubBrowse,
  clawhubSearch,
  clawhubGetSkill,
  skillhubBrowse,
  skillhubSearch,
  skillhubGetSkill,
  fanghubListSkills,
  // workflows
  listWorkflows,
  listWorkflowRuns,
  getWorkflowRun,
  listWorkflowTemplates,
  // terminal
  listTerminalWindows,
  // auto-dream
  getAutoDreamStatus,
} from "../../api";

// ---------------------------------------------------------------------------
// Mutation functions (write)
// ---------------------------------------------------------------------------
export {
  // agents
  spawnAgent,
  cloneAgent,
  suspendAgent,
  resumeAgent,
  deleteAgent,
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
  updateTrigger,
  deleteTrigger,
  // skills
  installSkill,
  uninstallSkill,
  clawhubInstall,
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
  MemoryItem,
  ModelOverrides,
  TerminalWindow,
} from "../../api";
