import { describe, expect, it } from "vitest";
import {
  agentKeys,
  modelKeys,
  handKeys,
  workflowKeys,
  auditKeys,
  configKeys,
  approvalKeys,
  memoryKeys,
  channelKeys,
  providerKeys,
  runtimeKeys,
  overviewKeys,
  scheduleKeys,
  triggerKeys,
  cronKeys,
  usageKeys,
  budgetKeys,
  goalKeys,
  networkKeys,
  peerKeys,
  a2aKeys,
  sessionKeys,
  mediaKeys,
  mcpKeys,
  pluginKeys,
  registryKeys,
  telemetryKeys,
  terminalKeys,
  commsKeys,
  skillKeys,
  clawhubKeys,
  skillhubKeys,
  fanghubKeys,
  totpKeys,
} from "./keys";

describe("query key factories", () => {
  describe("agentKeys", () => {
    it("generates hierarchical keys", () => {
      expect(agentKeys.all).toEqual(["agents"]);
      expect(agentKeys.lists()).toEqual(["agents", "list"]);
      expect(agentKeys.list({ includeHands: true })).toEqual([
        "agents",
        "list",
        { includeHands: true },
      ]);
      expect(agentKeys.list()).toEqual(["agents", "list", {}]);
      expect(agentKeys.details()).toEqual(["agents", "detail"]);
      expect(agentKeys.detail("abc")).toEqual(["agents", "detail", "abc"]);
      expect(agentKeys.templates()).toEqual(["agents", "templates"]);
      expect(agentKeys.sessions("abc")).toEqual([
        "agents",
        "sessions",
        "abc",
      ]);
    });

    it("detail is nested under details", () => {
      const d = agentKeys.detail("x");
      const ds = agentKeys.details();
      expect(d.slice(0, ds.length)).toEqual(ds);
    });

    it("list is nested under lists", () => {
      const l = agentKeys.list({ includeHands: false });
      const ls = agentKeys.lists();
      expect(l.slice(0, ls.length)).toEqual(ls);
    });
  });

  describe("modelKeys", () => {
    it("handles filters", () => {
      expect(modelKeys.list()).toEqual(["models", "list", {}]);
      expect(modelKeys.list({ provider: "openai" })).toEqual([
        "models",
        "list",
        { provider: "openai" },
      ]);
      expect(
        modelKeys.list({ provider: "openai", available: true }),
      ).toEqual([
        "models",
        "list",
        { provider: "openai", available: true },
      ]);
    });

    it("overrides are per model key", () => {
      expect(modelKeys.overrides("gpt-4")).toEqual([
        "models",
        "overrides",
        "gpt-4",
      ]);
    });
  });

  describe("handKeys", () => {
    it("stats vs statsBatch are different keys", () => {
      const single = handKeys.stats("inst-1");
      const batch = handKeys.statsBatch(["inst-1", "inst-2"]);
      expect(single).not.toEqual(batch);
      expect(single).toEqual(["hands", "stats", "inst-1"]);
      expect(batch).toEqual(["hands", "statsBatch", ["inst-1", "inst-2"]]);
    });

    it("active has no args", () => {
      expect(handKeys.active()).toEqual(["hands", "active"]);
      // Called twice — same reference
      expect(handKeys.active()).toEqual(handKeys.active());
    });
  });

  describe("workflowKeys", () => {
    it("templates with filters", () => {
      expect(workflowKeys.templates()).toEqual(["workflows", "templates", {}]);
      expect(workflowKeys.templates({ q: "deploy" })).toEqual([
        "workflows",
        "templates",
        { q: "deploy" },
      ]);
    });

    it("runs are per workflow", () => {
      expect(workflowKeys.runs("wf-1")).toEqual([
        "workflows",
        "runs",
        "wf-1",
      ]);
    });

    it("detail and runDetail are nested under their parent prefixes", () => {
      expect(
        workflowKeys.detail("wf-1").slice(0, workflowKeys.details().length),
      ).toEqual(workflowKeys.details());
      expect(
        workflowKeys.runDetail("run-1").slice(0, workflowKeys.runDetails().length),
      ).toEqual(workflowKeys.runDetails());
    });
  });

  describe("auditKeys", () => {
    it("recent includes limit in key (fixes RuntimePage bug)", () => {
      expect(auditKeys.recent(20)).toEqual(["audit", "recent", 20]);
      expect(auditKeys.recent(100)).toEqual(["audit", "recent", 100]);
      // Different limits = different keys
      expect(auditKeys.recent(20)).not.toEqual(auditKeys.recent(100));
    });
  });

  describe("configKeys", () => {
    it("full uses consistent key (fixes ChatPage/ConfigPage mismatch)", () => {
      expect(configKeys.full()).toEqual(["config", "full"]);
      // Always same
      expect(configKeys.full()).toEqual(configKeys.full());
    });
  });

  describe("approvalKeys", () => {
    it("pending filters by agent", () => {
      expect(approvalKeys.pending(null)).toEqual([
        "approvals",
        "pending",
        null,
      ]);
      expect(approvalKeys.pending("agent-1")).toEqual([
        "approvals",
        "pending",
        "agent-1",
      ]);
    });
  });

  describe("memoryKeys", () => {
    it("list with filters", () => {
      expect(memoryKeys.list()).toEqual(["memory", "list", {}]);
      expect(memoryKeys.list({ agentId: "a1", limit: 20 })).toEqual([
        "memory",
        "list",
        { agentId: "a1", limit: 20 },
      ]);
    });

    it("stats is per agent or global", () => {
      expect(memoryKeys.stats()).toEqual(["memory", "stats", undefined]);
      expect(memoryKeys.stats("a1")).toEqual(["memory", "stats", "a1"]);
    });
  });

  describe("structural stability", () => {
    it("same call returns structurally equal value", () => {
      const a = agentKeys.list({ includeHands: true });
      const b = agentKeys.list({ includeHands: true });
      expect(a).toEqual(b);
    });

    it("different filters produce different keys", () => {
      const a = modelKeys.list({ provider: "openai" });
      const b = modelKeys.list({ provider: "anthropic" });
      expect(a).not.toEqual(b);
    });
  });

  describe("invalidation patterns", () => {
    it("agentKeys.all prefixes all agent sub-keys", () => {
      const prefix = agentKeys.all;
      expect(agentKeys.lists().slice(0, prefix.length)).toEqual(prefix);
      expect(agentKeys.details().slice(0, prefix.length)).toEqual(prefix);
      expect(agentKeys.templates().slice(0, prefix.length)).toEqual(prefix);
      expect(agentKeys.sessions("x").slice(0, prefix.length)).toEqual(
        prefix,
      );
    });

    it("lists() prefixes list(filters)", () => {
      const ls = agentKeys.lists();
      const l = agentKeys.list({ includeHands: true });
      expect(l.slice(0, ls.length)).toEqual(ls);
    });
  });

  describe("skillKeys", () => {
    it("supporting files are nested under detail", () => {
      expect(skillKeys.supportingFiles("demo")).toEqual([
        "skills",
        "detail",
        "demo",
        "supportingFile",
      ]);
      expect(skillKeys.supportingFile("demo", "references/file.md")).toEqual([
        "skills",
        "detail",
        "demo",
        "supportingFile",
        "references/file.md",
      ]);
      expect(
        skillKeys.supportingFile("demo", "references/file.md").slice(
          0,
          skillKeys.supportingFiles("demo").length,
        ),
      ).toEqual(skillKeys.supportingFiles("demo"));
    });
  });

  describe("runtimeKeys anchoring", () => {
    it("all sub-keys are prefixed with runtimeKeys.all", () => {
      const prefix = runtimeKeys.all;
      expect(runtimeKeys.status().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.queueStatus().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.healthDetail().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.security().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.backups().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.tasks().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.taskStatus().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.taskList().slice(0, prefix.length)).toEqual(prefix);
      expect(runtimeKeys.taskList("running").slice(0, prefix.length)).toEqual(
        prefix,
      );
    });

    it("taskStatus and taskList share the tasks() prefix", () => {
      const tasksPrefix = runtimeKeys.tasks();
      expect(runtimeKeys.taskStatus().slice(0, tasksPrefix.length)).toEqual(
        tasksPrefix,
      );
      expect(runtimeKeys.taskList().slice(0, tasksPrefix.length)).toEqual(
        tasksPrefix,
      );
      expect(
        runtimeKeys.taskList("running").slice(0, tasksPrefix.length),
      ).toEqual(tasksPrefix);
    });
  });

  describe("overviewKeys anchoring", () => {
    it("version is prefixed with overviewKeys.all", () => {
      const prefix = overviewKeys.all;
      expect(overviewKeys.version().slice(0, prefix.length)).toEqual(prefix);
      expect(overviewKeys.snapshot().slice(0, prefix.length)).toEqual(prefix);
    });
  });

  describe("mediaKeys", () => {
    it("videoTask is nested under videoTasks and media all", () => {
      expect(mediaKeys.providers()).toEqual(["media", "providers"]);
      expect(mediaKeys.videoTasks()).toEqual(["media", "videoTasks"]);
      expect(mediaKeys.videoTask("task-1", "fal")).toEqual([
        "media",
        "videoTasks",
        "task-1",
        "fal",
      ]);

      const taskPrefix = mediaKeys.videoTasks();
      expect(mediaKeys.videoTask("task-1", "fal").slice(0, taskPrefix.length)).toEqual(taskPrefix);
      expect(mediaKeys.videoTasks().slice(0, mediaKeys.all.length)).toEqual(mediaKeys.all);
    });
  });

  describe("telemetryKeys", () => {
    it("metrics is prefixed with telemetryKeys.all", () => {
      const prefix = telemetryKeys.all;
      expect(telemetryKeys.metrics().slice(0, prefix.length)).toEqual(prefix);
      expect(telemetryKeys.metrics()).toEqual(["telemetry", "metrics"]);
    });
  });

  describe("all factories exist", () => {
    const factories = [
      agentKeys,
      modelKeys,
      providerKeys,
      channelKeys,
      commsKeys,
      skillKeys,
      clawhubKeys,
      skillhubKeys,
      fanghubKeys,
      handKeys,
      workflowKeys,
      scheduleKeys,
      triggerKeys,
      cronKeys,
      approvalKeys,
      totpKeys,
      memoryKeys,
      usageKeys,
      budgetKeys,
      goalKeys,
      networkKeys,
      peerKeys,
      a2aKeys,
      sessionKeys,
      overviewKeys,
      runtimeKeys,
      auditKeys,
      mediaKeys,
      mcpKeys,
      pluginKeys,
      configKeys,
      registryKeys,
      telemetryKeys,
      terminalKeys,
    ];

    it("all factories have an 'all' key", () => {
      for (const f of factories) {
        expect(f.all).toBeDefined();
        expect(Array.isArray(f.all)).toBe(true);
        expect((f.all as readonly string[]).length).toBeGreaterThan(0);
      }
    });
  });

  describe("terminalKeys anchoring", () => {
    it("health and windows are prefixed with terminalKeys.all", () => {
      const prefix = terminalKeys.all;
      expect(terminalKeys.health().slice(0, prefix.length)).toEqual(prefix);
      expect(terminalKeys.windows().slice(0, prefix.length)).toEqual(prefix);
    });
  });
});
