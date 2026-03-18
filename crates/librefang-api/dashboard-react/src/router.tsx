import { Navigate, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { createHashHistory } from "@tanstack/history";
import { App } from "./App";
import { AgentsPage } from "./pages/AgentsPage";
import { AnalyticsPage } from "./pages/AnalyticsPage";
import { ApprovalsPage } from "./pages/ApprovalsPage";
import { ChannelsPage } from "./pages/ChannelsPage";
import { CommsPage } from "./pages/CommsPage";
import { GoalsPage } from "./pages/GoalsPage";
import { HandsPage } from "./pages/HandsPage";
import { LogsPage } from "./pages/LogsPage";
import { MemoryPage } from "./pages/MemoryPage";
import { OverviewPage } from "./pages/OverviewPage";
import { ProvidersPage } from "./pages/ProvidersPage";
import { RuntimePage } from "./pages/RuntimePage";
import { SchedulerPage } from "./pages/SchedulerPage";
import { SessionsPage } from "./pages/SessionsPage";
import { SkillsPage } from "./pages/SkillsPage";
import { WorkflowsPage } from "./pages/WorkflowsPage";

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
  component: OverviewPage
});

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: AgentsPage
});

const sessionsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/sessions",
  component: SessionsPage
});

const providersRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/providers",
  component: ProvidersPage
});

const channelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/channels",
  component: ChannelsPage
});

const skillsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/skills",
  component: SkillsPage
});

const workflowsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workflows",
  component: WorkflowsPage
});

const schedulerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/scheduler",
  component: SchedulerPage
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/analytics",
  component: AnalyticsPage
});

const memoryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/memory",
  component: MemoryPage
});

const runtimeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runtime",
  component: RuntimePage
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: LogsPage
});

const approvalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/approvals",
  component: ApprovalsPage
});

const commsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/comms",
  component: CommsPage
});

const handsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/hands",
  component: HandsPage
});

const goalsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/goals",
  component: GoalsPage
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  overviewRoute,
  agentsRoute,
  sessionsRoute,
  providersRoute,
  channelsRoute,
  skillsRoute,
  workflowsRoute,
  schedulerRoute,
  analyticsRoute,
  memoryRoute,
  runtimeRoute,
  logsRoute,
  approvalsRoute,
  commsRoute,
  handsRoute,
  goalsRoute
]);

export const router = createRouter({
  routeTree,
  history: createHashHistory()
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
