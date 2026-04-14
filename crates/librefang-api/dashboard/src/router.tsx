import { lazy, Suspense } from "react";
import { Navigate, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { createHashHistory } from "@tanstack/history";
import { App } from "./App";

// Lazy-loaded pages — each becomes a separate chunk
const OverviewPage = lazy(() => import("./pages/OverviewPage").then(m => ({ default: m.OverviewPage })));
const AgentsPage = lazy(() => import("./pages/AgentsPage").then(m => ({ default: m.AgentsPage })));
const AnalyticsPage = lazy(() => import("./pages/AnalyticsPage").then(m => ({ default: m.AnalyticsPage })));
const CanvasPage = lazy(() => import("./pages/CanvasPage").then(m => ({ default: m.CanvasPage })));
const ApprovalsPage = lazy(() => import("./pages/ApprovalsPage").then(m => ({ default: m.ApprovalsPage })));
const ChannelsPage = lazy(() => import("./pages/ChannelsPage").then(m => ({ default: m.ChannelsPage })));
const ChatPage = lazy(() => import("./pages/ChatPage").then(m => ({ default: m.ChatPage })));
const CommsPage = lazy(() => import("./pages/CommsPage").then(m => ({ default: m.CommsPage })));
const GoalsPage = lazy(() => import("./pages/GoalsPage").then(m => ({ default: m.GoalsPage })));
const HandsPage = lazy(() => import("./pages/HandsPage").then(m => ({ default: m.HandsPage })));
const LogsPage = lazy(() => import("./pages/LogsPage").then(m => ({ default: m.LogsPage })));
const MemoryPage = lazy(() => import("./pages/MemoryPage").then(m => ({ default: m.MemoryPage })));
const ProvidersPage = lazy(() => import("./pages/ProvidersPage").then(m => ({ default: m.ProvidersPage })));
const RuntimePage = lazy(() => import("./pages/RuntimePage").then(m => ({ default: m.RuntimePage })));
const SchedulerPage = lazy(() => import("./pages/SchedulerPage").then(m => ({ default: m.SchedulerPage })));
const SessionsPage = lazy(() => import("./pages/SessionsPage").then(m => ({ default: m.SessionsPage })));
const SettingsPage = lazy(() => import("./pages/SettingsPage").then(m => ({ default: m.SettingsPage })));
const SkillsPage = lazy(() => import("./pages/SkillsPage").then(m => ({ default: m.SkillsPage })));
const WizardPage = lazy(() => import("./pages/WizardPage").then(m => ({ default: m.WizardPage })));
const WorkflowsPage = lazy(() => import("./pages/WorkflowsPage").then(m => ({ default: m.WorkflowsPage })));
const PluginsPage = lazy(() => import("./pages/PluginsPage").then(m => ({ default: m.PluginsPage })));
const ModelsPage = lazy(() => import("./pages/ModelsPage").then(m => ({ default: m.ModelsPage })));
const MediaPage = lazy(() => import("./pages/MediaPage").then(m => ({ default: m.MediaPage })));
const NetworkPage = lazy(() => import("./pages/NetworkPage").then(m => ({ default: m.NetworkPage })));
const A2APage = lazy(() => import("./pages/A2APage").then(m => ({ default: m.A2APage })));
const TelemetryPage = lazy(() => import("./pages/TelemetryPage").then(m => ({ default: m.TelemetryPage })));
const TerminalPage = lazy(() => import("./pages/TerminalPage").then(m => ({ default: m.TerminalPage })));
const McpServersPage = lazy(() => import("./pages/McpServersPage").then(m => ({ default: m.McpServersPage })));
const ConfigPage = lazy(() => import("./pages/ConfigPage").then(m => ({ default: m.ConfigPage })));

// Suspense wrapper — shows nothing briefly while chunk loads (page transition animation covers it)
function L({ children }: { children: React.ReactNode }) {
  return <Suspense fallback={null}>{children}</Suspense>;
}

const rootRoute = createRootRoute({
  component: App
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <Navigate to="/overview" />
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/overview",
  component: () => <L><OverviewPage /></L>
});

const canvasRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/canvas",
  validateSearch: (search: Record<string, unknown>) => ({
    t: search.t as number | undefined,
    wf: search.wf as string | undefined,
  }),
  component: () => <L><CanvasPage /></L>
});

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: () => <L><AgentsPage /></L>
});

const sessionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sessions",
  component: () => <L><SessionsPage /></L>
});

const providersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/providers",
  component: () => <L><ProvidersPage /></L>
});

const channelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/channels",
  component: () => <L><ChannelsPage /></L>
});

const chatRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/chat",
  validateSearch: (search: Record<string, unknown>) => ({
    agentId: search.agentId as string | undefined
  }),
  component: () => <L><ChatPage /></L>
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: () => <L><SettingsPage /></L>
});

const skillsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/skills",
  component: () => <L><SkillsPage /></L>
});

const wizardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/wizard",
  component: () => <L><WizardPage /></L>
});

const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  component: () => <L><WorkflowsPage /></L>
});

const schedulerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scheduler",
  component: () => <L><SchedulerPage /></L>
});

const goalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/goals",
  component: () => <L><GoalsPage /></L>
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/analytics",
  component: () => <L><AnalyticsPage /></L>
});

const memoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/memory",
  component: () => <L><MemoryPage /></L>
});

const commsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/comms",
  component: () => <L><CommsPage /></L>
});

const runtimeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runtime",
  component: () => <L><RuntimePage /></L>
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: () => <L><LogsPage /></L>
});

const approvalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/approvals",
  component: () => <L><ApprovalsPage /></L>
});

const handsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/hands",
  component: () => <L><HandsPage /></L>
});

const pluginsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plugins",
  component: () => <L><PluginsPage /></L>
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/models",
  component: () => <L><ModelsPage /></L>
});

const mediaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/media",
  component: () => <L><MediaPage /></L>
});

const networkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/network",
  component: () => <L><NetworkPage /></L>
});

const a2aRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/a2a",
  component: () => <L><A2APage /></L>
});

const telemetryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/telemetry",
  component: () => <L><TelemetryPage /></L>
});

const terminalRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/terminal",
  component: () => <L><TerminalPage /></L>
});
const mcpServersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp-servers",
  component: () => <L><McpServersPage /></L>
});

const configIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config",
  component: () => <Navigate to="/config/general" />
});
const configGeneralRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/general",
  component: () => <L><ConfigPage category="general" /></L>
});
const configMemoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/memory",
  component: () => <L><ConfigPage category="memory" /></L>
});
const configToolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/tools",
  component: () => <L><ConfigPage category="tools" /></L>
});
const configChannelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/channels",
  component: () => <L><ConfigPage category="channels" /></L>
});
const configSecurityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/security",
  component: () => <L><ConfigPage category="security" /></L>
});
const configNetworkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/network",
  component: () => <L><ConfigPage category="network" /></L>
});
const configInfraRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/infra",
  component: () => <L><ConfigPage category="infra" /></L>
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  overviewRoute,
  canvasRoute,
  agentsRoute,
  sessionsRoute,
  providersRoute,
  channelsRoute,
  chatRoute,
  settingsRoute,
  skillsRoute,
  wizardRoute,
  workflowsRoute,
  schedulerRoute,
  goalsRoute,
  analyticsRoute,
  memoryRoute,
  commsRoute,
  runtimeRoute,
  logsRoute,
  approvalsRoute,
  handsRoute,
  pluginsRoute,
  modelsRoute,
  mediaRoute,
  networkRoute,
  a2aRoute,
  telemetryRoute,
  terminalRoute,
  mcpServersRoute,
  configIndexRoute,
  configGeneralRoute,
  configMemoryRoute,
  configToolsRoute,
  configChannelsRoute,
  configSecurityRoute,
  configNetworkRoute,
  configInfraRoute,
]);

export const router = createRouter({
  routeTree,
  history: createHashHistory(),
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
