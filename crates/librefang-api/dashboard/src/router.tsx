import { lazy, Suspense, useEffect, useState, type ComponentType } from "react";
import { Navigate, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { App } from "./App";

// Matches chunk load failures across browsers:
// Chrome:  "Failed to fetch dynamically imported module: ..."
// Firefox: "error loading dynamically imported module: ..."
// Safari:  "Importing a module script failed"
// Webpack: "Loading chunk ... failed"
const CHUNK_ERROR_RE = /dynamically imported module|importing a module script|Loading chunk .* failed/i;

// Matches the transient React 19 + Vite HMR failure mode where the dispatcher
// is null at hook-read time. A full reload reliably clears the stale module
// graph; we auto-reload once per session so users don't have to click Reload.
const REACT_DISPATCHER_RE = /reading ['"]useContext['"]|reading ['"]useState['"]|reading ['"]useRef['"]|reading ['"]useMemo['"]|reading ['"]useEffect['"]|reading ['"]useReducer['"]|reading ['"]useCallback['"]|reading ['"]useLayoutEffect['"]|reading ['"]useSyncExternalStore['"]/;

function shouldAutoReload(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return CHUNK_ERROR_RE.test(msg) || REACT_DISPATCHER_RE.test(msg);
}

// Auto-reload on stale chunk — when the dashboard is rebuilt (dev HMR, sync,
// or version upgrade) the old chunk hashes no longer exist on the server.
// Detect the chunk error and reload once so the browser picks up the new
// index.html with correct chunk hashes. A sessionStorage guard prevents
// infinite reload loops.
function tryAutoReload(err: unknown): boolean {
  if (!shouldAutoReload(err)) return false;
  const key = "__chunk_reload";
  const last = Number(sessionStorage.getItem(key) || "0");
  if (Date.now() - last <= 10_000) return false;
  sessionStorage.setItem(key, String(Date.now()));
  window.location.reload();
  return true;
}

function lazyWithReload<T extends ComponentType<any>>(
  factory: () => Promise<{ default: T }>,
): React.LazyExoticComponent<T> {
  return lazy(() =>
    factory().catch((err: unknown) => {
      if (tryAutoReload(err)) {
        // Return a never-resolving promise so React doesn't render the
        // error boundary before the reload takes effect.
        return new Promise<never>(() => {});
      }
      throw err;
    }),
  );
}

// Lazy-loaded pages — each becomes a separate chunk
const OverviewPage = lazyWithReload(() => import("./pages/OverviewPage").then(m => ({ default: m.OverviewPage })));
const AgentsPage = lazyWithReload(() => import("./pages/AgentsPage").then(m => ({ default: m.AgentsPage })));
const AnalyticsPage = lazyWithReload(() => import("./pages/AnalyticsPage").then(m => ({ default: m.AnalyticsPage })));
const CanvasPage = lazyWithReload(() => import("./pages/CanvasPage").then(m => ({ default: m.CanvasPage })));
const ApprovalsPage = lazyWithReload(() => import("./pages/ApprovalsPage").then(m => ({ default: m.ApprovalsPage })));
const ChannelsPage = lazyWithReload(() => import("./pages/ChannelsPage").then(m => ({ default: m.ChannelsPage })));
const ChatPage = lazyWithReload(() => import("./pages/ChatPage").then(m => ({ default: m.ChatPage })));
const CommsPage = lazyWithReload(() => import("./pages/CommsPage").then(m => ({ default: m.CommsPage })));
const GoalsPage = lazyWithReload(() => import("./pages/GoalsPage").then(m => ({ default: m.GoalsPage })));
const HandsPage = lazyWithReload(() => import("./pages/HandsPage").then(m => ({ default: m.HandsPage })));
const LogsPage = lazyWithReload(() => import("./pages/LogsPage").then(m => ({ default: m.LogsPage })));
const MemoryPage = lazyWithReload(() => import("./pages/MemoryPage").then(m => ({ default: m.MemoryPage })));
const ProvidersPage = lazyWithReload(() => import("./pages/ProvidersPage").then(m => ({ default: m.ProvidersPage })));
const RuntimePage = lazyWithReload(() => import("./pages/RuntimePage").then(m => ({ default: m.RuntimePage })));
const SchedulerPage = lazyWithReload(() => import("./pages/SchedulerPage").then(m => ({ default: m.SchedulerPage })));
const SessionsPage = lazyWithReload(() => import("./pages/SessionsPage").then(m => ({ default: m.SessionsPage })));
const SettingsPage = lazyWithReload(() => import("./pages/SettingsPage").then(m => ({ default: m.SettingsPage })));
const SkillsPage = lazyWithReload(() => import("./pages/SkillsPage").then(m => ({ default: m.SkillsPage })));
const WizardPage = lazyWithReload(() => import("./pages/WizardPage").then(m => ({ default: m.WizardPage })));
const WorkflowsPage = lazyWithReload(() => import("./pages/WorkflowsPage").then(m => ({ default: m.WorkflowsPage })));
const PluginsPage = lazyWithReload(() => import("./pages/PluginsPage").then(m => ({ default: m.PluginsPage })));
const ModelsPage = lazyWithReload(() => import("./pages/ModelsPage").then(m => ({ default: m.ModelsPage })));
const MediaPage = lazyWithReload(() => import("./pages/MediaPage").then(m => ({ default: m.MediaPage })));
const NetworkPage = lazyWithReload(() => import("./pages/NetworkPage").then(m => ({ default: m.NetworkPage })));
const A2APage = lazyWithReload(() => import("./pages/A2APage").then(m => ({ default: m.A2APage })));
const TelemetryPage = lazyWithReload(() => import("./pages/TelemetryPage").then(m => ({ default: m.TelemetryPage })));
const TerminalPage = lazyWithReload(() => import("./pages/TerminalPage").then(m => ({ default: m.TerminalPage })));
const McpServersPage = lazyWithReload(() => import("./pages/McpServersPage").then(m => ({ default: m.McpServersPage })));
const ConfigPage = lazyWithReload(() => import("./pages/ConfigPage").then(m => ({ default: m.ConfigPage })));

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

function ChunkErrorBoundary({ error }: { error: Error }) {
  const isChunkError = CHUNK_ERROR_RE.test(error.message);
  const isDispatcherError = REACT_DISPATCHER_RE.test(error.message);
  const [showStack, setShowStack] = useState(false);

  // Auto-reload once per session for known-transient failures (chunk misses,
  // React dispatcher-null after HMR). If the reload fires we never render
  // past this effect; otherwise we show the diagnostic UI below.
  useEffect(() => {
    if (isChunkError || isDispatcherError) {
      tryAutoReload(error);
    }
  }, [error, isChunkError, isDispatcherError]);

  const title = isChunkError
    ? "Page assets have been updated"
    : isDispatcherError
    ? "React state reset — reloading"
    : "Something went wrong";
  const detail = isChunkError
    ? "A new version is available. Reload to get the latest."
    : error.message;

  return (
    <div className="flex h-[60vh] items-center justify-center">
      <div className="max-w-xl text-center space-y-4 px-4">
        <p className="text-lg font-semibold">{title}</p>
        <p className="text-sm text-gray-500 break-words">{detail}</p>
        <div className="flex gap-2 justify-center flex-wrap">
          <button
            onClick={() => window.location.reload()}
            className="rounded-xl bg-sky-500 px-6 py-2.5 text-sm font-bold text-white hover:bg-sky-600 transition-colors"
          >
            Reload
          </button>
          <button
            onClick={() => {
              sessionStorage.removeItem("__chunk_reload");
              window.location.reload();
            }}
            className="rounded-xl bg-red-500 px-6 py-2.5 text-sm font-bold text-white hover:bg-red-600 transition-colors"
            title="Clears the auto-reload cooldown and forces a fresh load"
          >
            Force reload
          </button>
          {error.stack && (
            <button
              onClick={() => setShowStack(v => !v)}
              className="rounded-xl border border-gray-300 px-6 py-2.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
            >
              {showStack ? "Hide" : "Show"} stack
            </button>
          )}
        </div>
        {showStack && error.stack && (
          <pre className="mt-4 max-h-64 overflow-auto rounded-lg bg-gray-900 p-3 text-left text-xs text-gray-100 whitespace-pre-wrap break-all">
            {error.stack}
          </pre>
        )}
      </div>
    </div>
  );
}

export const router = createRouter({
  routeTree,
  basepath: "/dashboard",
  defaultPreload: "intent",
  defaultErrorComponent: ChunkErrorBoundary as any,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
