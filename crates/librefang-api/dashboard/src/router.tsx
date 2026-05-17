import { lazy, Suspense, useEffect, useState, type ComponentType } from "react";
import { Link, Navigate, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { App } from "./App";

// Matches chunk load failures across browsers:
// Chrome:  "Failed to fetch dynamically imported module: ..."
// Firefox: "error loading dynamically imported module: ..."
// Safari:  "Importing a module script failed"
// Webpack: "Loading chunk ... failed"
const CHUNK_RELOAD_KEY = "__chunk_reload";

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
  const last = Number(sessionStorage.getItem(CHUNK_RELOAD_KEY) || "0");
  if (Date.now() - last <= 10_000) return false;
  sessionStorage.setItem(CHUNK_RELOAD_KEY, String(Date.now()));
  window.location.reload();
  return true;
}

// `ComponentType<any>` mirrors React's own `lazy<T extends ComponentType<any>>`
// signature. Contravariance on the props parameter means narrower types
// (`ComponentType<{}>` / `<unknown>` / `<never>`) reject lazy targets that
// have required props, defeating the purpose of the wrapper.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
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
const MemoryPage = lazyWithReload(() => import("./pages/Memory").then(m => ({ default: m.MemoryPage })));
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
const UsersPage = lazyWithReload(() => import("./pages/UsersPage").then(m => ({ default: m.UsersPage })));
const PermissionSimulatorPage = lazyWithReload(() => import("./pages/PermissionSimulatorPage").then(m => ({ default: m.PermissionSimulatorPage })));
const AuditPage = lazyWithReload(() => import("./pages/AuditPage").then(m => ({ default: m.AuditPage })));
const UserBudgetPage = lazyWithReload(() => import("./pages/UserBudgetPage").then(m => ({ default: m.UserBudgetPage })));
const UserPolicyPage = lazyWithReload(() => import("./pages/UserPolicyPage").then(m => ({ default: m.UserPolicyPage })));
const ConnectWizardPage = lazyWithReload(() => import("./pages/ConnectWizardPage").then(m => ({ default: m.ConnectWizardPage })));
const MobilePairingPage = lazyWithReload(() => import("./pages/MobilePairingPage").then(m => ({ default: m.MobilePairingPage })));

function LazyRouteBoundary({ children }: { children: React.ReactNode }) {
  return (
    <Suspense
      fallback={
        <div className="flex h-32 items-center justify-center">
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-gray-300 border-t-sky-500" />
        </div>
      }
    >
      {children}
    </Suspense>
  );
}

const rootRoute = createRootRoute({
  component: App,
  // Explicit handler for notFound() bubbling to `__root__` (unmatched
  // URLs, or notFound thrown in a loader/beforeLoad). Without this the
  // router falls back to its generic `<p>Not Found</p>` and logs a dev
  // warning; `defaultNotFoundComponent` below is the global fallback,
  // this is the root-route-level handler the warning asks for.
  notFoundComponent: NotFound,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <Navigate to="/overview" />
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/overview",
  component: () => <LazyRouteBoundary><OverviewPage /></LazyRouteBoundary>
});

const canvasRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/canvas",
  validateSearch: (search: Record<string, unknown>) => ({
    t: search.t as number | undefined,
    wf: search.wf as string | undefined,
  }),
  component: () => <LazyRouteBoundary><CanvasPage /></LazyRouteBoundary>
});

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: () => <LazyRouteBoundary><AgentsPage /></LazyRouteBoundary>
});

const sessionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sessions",
  component: () => <LazyRouteBoundary><SessionsPage /></LazyRouteBoundary>
});

const providersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/providers",
  component: () => <LazyRouteBoundary><ProvidersPage /></LazyRouteBoundary>
});

const channelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/channels",
  component: () => <LazyRouteBoundary><ChannelsPage /></LazyRouteBoundary>
});

const chatRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/chat",
  validateSearch: (search: Record<string, unknown>): { agentId?: string; sessionId?: string; attach?: string; handName?: string } => {
    const out: { agentId?: string; sessionId?: string; attach?: string; handName?: string } = {};
    if (typeof search.agentId === "string") out.agentId = search.agentId;
    if (typeof search.sessionId === "string") out.sessionId = search.sessionId;
    if (typeof search.attach === "string") out.attach = search.attach;
    if (typeof search.handName === "string") out.handName = search.handName;
    return out;
  },
  component: () => <LazyRouteBoundary><ChatPage /></LazyRouteBoundary>
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: () => <LazyRouteBoundary><SettingsPage /></LazyRouteBoundary>
});

const skillsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/skills",
  component: () => <LazyRouteBoundary><SkillsPage /></LazyRouteBoundary>
});

const wizardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/wizard",
  component: () => <LazyRouteBoundary><WizardPage /></LazyRouteBoundary>
});

const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  component: () => <LazyRouteBoundary><WorkflowsPage /></LazyRouteBoundary>
});

const schedulerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scheduler",
  component: () => <LazyRouteBoundary><SchedulerPage /></LazyRouteBoundary>
});

const goalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/goals",
  component: () => <LazyRouteBoundary><GoalsPage /></LazyRouteBoundary>
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/analytics",
  component: () => <LazyRouteBoundary><AnalyticsPage /></LazyRouteBoundary>
});

const memoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/memory",
  validateSearch: (search: Record<string, unknown>): { agent?: string; tab?: "records" | "kv" | "dreams" | "health" } => {
    const out: { agent?: string; tab?: "records" | "kv" | "dreams" | "health" } = {};
    if (typeof search.agent === "string") out.agent = search.agent;
    if (search.tab === "records" || search.tab === "kv" || search.tab === "dreams" || search.tab === "health") {
      out.tab = search.tab;
    }
    return out;
  },
  component: () => <LazyRouteBoundary><MemoryPage /></LazyRouteBoundary>
});

const commsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/comms",
  component: () => <LazyRouteBoundary><CommsPage /></LazyRouteBoundary>
});

const runtimeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runtime",
  component: () => <LazyRouteBoundary><RuntimePage /></LazyRouteBoundary>
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: () => <LazyRouteBoundary><LogsPage /></LazyRouteBoundary>
});

const approvalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/approvals",
  component: () => <LazyRouteBoundary><ApprovalsPage /></LazyRouteBoundary>
});

const handsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/hands",
  component: () => <LazyRouteBoundary><HandsPage /></LazyRouteBoundary>
});

const pluginsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/plugins",
  component: () => <LazyRouteBoundary><PluginsPage /></LazyRouteBoundary>
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/models",
  component: () => <LazyRouteBoundary><ModelsPage /></LazyRouteBoundary>
});

const mediaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/media",
  component: () => <LazyRouteBoundary><MediaPage /></LazyRouteBoundary>
});

const networkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/network",
  component: () => <LazyRouteBoundary><NetworkPage /></LazyRouteBoundary>
});

const a2aRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/a2a",
  component: () => <LazyRouteBoundary><A2APage /></LazyRouteBoundary>
});

const telemetryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/telemetry",
  component: () => <LazyRouteBoundary><TelemetryPage /></LazyRouteBoundary>
});

const terminalRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/terminal",
  component: () => <LazyRouteBoundary><TerminalPage /></LazyRouteBoundary>
});
const mcpServersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/mcp-servers",
  component: () => <LazyRouteBoundary><McpServersPage /></LazyRouteBoundary>
});

// RBAC M6 — users, identity wizard, permission simulator. Per-user budget
// and per-user policy pages stub the M3/M5 endpoints; the route is live so
// query hooks stay wired.
const usersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users",
  component: () => <LazyRouteBoundary><UsersPage /></LazyRouteBoundary>
});
const usersSimulatorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users/simulator",
  component: () => <LazyRouteBoundary><PermissionSimulatorPage /></LazyRouteBoundary>
});
const userBudgetRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users/$name/budget",
  component: () => <LazyRouteBoundary><UserBudgetPage /></LazyRouteBoundary>
});
const userPolicyRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/users/$name/policy",
  component: () => <LazyRouteBoundary><UserPolicyPage /></LazyRouteBoundary>
});
const auditRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/audit",
  component: () => <LazyRouteBoundary><AuditPage /></LazyRouteBoundary>
});

const connectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/connect",
  component: () => <LazyRouteBoundary><ConnectWizardPage /></LazyRouteBoundary>
});

const mobilePairingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/mobile-pairing",
  component: () => <LazyRouteBoundary><MobilePairingPage /></LazyRouteBoundary>
});

const configIndexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config",
  component: () => <Navigate to="/config/general" />
});
const configGeneralRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/general",
  component: () => <LazyRouteBoundary><ConfigPage category="general" /></LazyRouteBoundary>
});
const configMemoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/memory",
  component: () => <LazyRouteBoundary><ConfigPage category="memory" /></LazyRouteBoundary>
});
const configToolsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/tools",
  component: () => <LazyRouteBoundary><ConfigPage category="tools" /></LazyRouteBoundary>
});
const configChannelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/channels",
  component: () => <LazyRouteBoundary><ConfigPage category="channels" /></LazyRouteBoundary>
});
const configSecurityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/security",
  component: () => <LazyRouteBoundary><ConfigPage category="security" /></LazyRouteBoundary>
});
const configNetworkRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/network",
  component: () => <LazyRouteBoundary><ConfigPage category="network" /></LazyRouteBoundary>
});
const configInfraRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/config/infra",
  component: () => <LazyRouteBoundary><ConfigPage category="infra" /></LazyRouteBoundary>
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
  usersRoute,
  usersSimulatorRoute,
  userBudgetRoute,
  userPolicyRoute,
  auditRoute,
  connectRoute,
  mobilePairingRoute,
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
              sessionStorage.removeItem(CHUNK_RELOAD_KEY);
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

function NotFound() {
  return (
    <div className="flex h-[60vh] items-center justify-center">
      <div className="max-w-xl text-center space-y-4 px-4">
        <p className="text-lg font-semibold">Page not found</p>
        <Link
          to="/overview"
          className="inline-block rounded-xl bg-sky-500 px-6 py-2.5 text-sm font-bold text-white hover:bg-sky-600 transition-colors"
        >
          Go to Overview
        </Link>
      </div>
    </div>
  );
}

export const router = createRouter({
  routeTree,
  basepath: "/dashboard",
  defaultPreload: "intent",
  defaultErrorComponent: ChunkErrorBoundary,
  defaultNotFoundComponent: NotFound,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
