/**
 * @librefang/sdk — AUTO-GENERATED from openapi.json.
 * Do not edit manually. Run: python3 scripts/codegen-sdks.py
 *
 * Usage:
 *   const { LibreFang } = require("@librefang/sdk");
 *   const client = new LibreFang("http://localhost:4545");
 *
 *   const agents = await client.agents.listAgents();
 *
 *   // Streaming:
 *   for await (const event of client.agents.sendMessageStream(agentId, { message: "Hello" })) {
 *     process.stdout.write(event.delta || "");
 *   }
 */

"use strict";

class LibreFangError extends Error {
  constructor(message, status, body) {
    super(message);
    this.name = "LibreFangError";
    this.status = status;
    this.body = body;
  }
}

class LibreFang {
  constructor(baseUrl, opts) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this._headers = Object.assign({ "Content-Type": "application/json" }, (opts && opts.headers) || {});
    this.a2a = new A2AResource(this);
    this.agents = new AgentsResource(this);
    this.approvals = new ApprovalsResource(this);
    this.auth = new AuthResource(this);
    this.auto_dream = new AutoDreamResource(this);
    this.budget = new BudgetResource(this);
    this.channels = new ChannelsResource(this);
    this.extensions = new ExtensionsResource(this);
    this.hands = new HandsResource(this);
    this.mcp = new McpResource(this);
    this.memory = new MemoryResource(this);
    this.models = new ModelsResource(this);
    this.network = new NetworkResource(this);
    this.pairing = new PairingResource(this);
    this.proactive_memory = new ProactiveMemoryResource(this);
    this.sessions = new SessionsResource(this);
    this.skills = new SkillsResource(this);
    this.system = new SystemResource(this);
    this.tools = new ToolsResource(this);
    this.webhooks = new WebhooksResource(this);
    this.workflows = new WorkflowsResource(this);
  }

  async _request(method, path, body) {
    const url = this.baseUrl + path;
    const opts = { method, headers: this._headers };
    if (body !== undefined) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    const text = await res.text();
    if (!res.ok) throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    const ct = res.headers.get("content-type") || "";
    return ct.includes("application/json") ? JSON.parse(text) : text;
  }

  async *_stream(method, path, body) {
    const url = this.baseUrl + path;
    const headers = Object.assign({}, this._headers, { Accept: "text/event-stream" });
    const opts = { method, headers };
    if (body !== undefined) opts.body = JSON.stringify(body);
    const res = await fetch(url, opts);
    if (!res.ok) {
      const text = await res.text();
      throw new LibreFangError(`HTTP ${res.status}: ${text}`, res.status, text);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop();
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith("data: ")) continue;
        const data = trimmed.slice(6);
        if (data === "[DONE]") return;
        try { yield JSON.parse(data); } catch { yield { raw: data }; }
      }
    }
  }
}

// ── A2A Resource

class A2AResource {
  constructor(client) { this._c = client; }

  async a2aListExternalAgents() {
    return this._c._request("GET", "/api/a2a/agents");
  }

  async a2aGetExternalAgent(id) {
    return this._c._request("GET", `/api/a2a/agents/${id}`);
  }

  async a2aDiscoverExternal(data) {
    return this._c._request("POST", "/api/a2a/discover", data);
  }

  async a2aSendExternal(data) {
    return this._c._request("POST", "/api/a2a/send", data);
  }

  async a2aExternalTaskStatus(id) {
    return this._c._request("GET", `/api/a2a/tasks/${id}/status`);
  }
}

// ── Agents Resource

class AgentsResource {
  constructor(client) { this._c = client; }

  async listAgents() {
    return this._c._request("GET", "/api/agents");
  }

  async spawnAgent(data) {
    return this._c._request("POST", "/api/agents", data);
  }

  async bulkCreateAgents(data) {
    return this._c._request("POST", "/api/agents/bulk", data);
  }

  async bulkDeleteAgents() {
    return this._c._request("DELETE", "/api/agents/bulk");
  }

  async bulkStartAgents(data) {
    return this._c._request("POST", "/api/agents/bulk/start", data);
  }

  async bulkStopAgents(data) {
    return this._c._request("POST", "/api/agents/bulk/stop", data);
  }

  async getAgent(id) {
    return this._c._request("GET", `/api/agents/${id}`);
  }

  async killAgent(id) {
    return this._c._request("DELETE", `/api/agents/${id}`);
  }

  async patchAgent(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}`, data);
  }

  async cloneAgent(id, data) {
    return this._c._request("POST", `/api/agents/${id}/clone`, data);
  }

  async patchAgentConfig(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}/config`, data);
  }

  async getAgentDeliveries(id) {
    return this._c._request("GET", `/api/agents/${id}/deliveries`);
  }

  async listAgentFiles(id) {
    return this._c._request("GET", `/api/agents/${id}/files`);
  }

  async getAgentFile(id, filename) {
    return this._c._request("GET", `/api/agents/${id}/files/${filename}`);
  }

  async setAgentFile(id, filename, data) {
    return this._c._request("PUT", `/api/agents/${id}/files/${filename}`, data);
  }

  async deleteAgentFile(id, filename) {
    return this._c._request("DELETE", `/api/agents/${id}/files/${filename}`);
  }

  async clearAgentHistory(id) {
    return this._c._request("DELETE", `/api/agents/${id}/history`);
  }

  async updateAgentIdentity(id, data) {
    return this._c._request("PATCH", `/api/agents/${id}/identity`, data);
  }

  async getAgentMcpServers(id) {
    return this._c._request("GET", `/api/agents/${id}/mcp_servers`);
  }

  async setAgentMcpServers(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/mcp_servers`, data);
  }

  async sendMessage(id, data) {
    return this._c._request("POST", `/api/agents/${id}/message`, data);
  }

  async *sendMessageStream(id, data) {
    yield* this._c._stream("POST", `/api/agents/${id}/message/stream`, data);
  }

  async setAgentMode(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/mode`, data);
  }

  async setModel(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/model`, data);
  }

  async getAgentSession(id) {
    return this._c._request("GET", `/api/agents/${id}/session`);
  }

  async compactSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/compact`);
  }

  async rebootSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/reboot`);
  }

  async resetSession(id) {
    return this._c._request("POST", `/api/agents/${id}/session/reset`);
  }

  async listAgentSessions(id) {
    return this._c._request("GET", `/api/agents/${id}/sessions`);
  }

  async createAgentSession(id, data) {
    return this._c._request("POST", `/api/agents/${id}/sessions`, data);
  }

  async importSession(id, data) {
    return this._c._request("POST", `/api/agents/${id}/sessions/import`, data);
  }

  async exportSession(id, session_id) {
    return this._c._request("GET", `/api/agents/${id}/sessions/${session_id}/export`);
  }

  async switchAgentSession(id, session_id) {
    return this._c._request("POST", `/api/agents/${id}/sessions/${session_id}/switch`);
  }

  async getAgentSkills(id) {
    return this._c._request("GET", `/api/agents/${id}/skills`);
  }

  async setAgentSkills(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/skills`, data);
  }

  async stopAgent(id) {
    return this._c._request("POST", `/api/agents/${id}/stop`);
  }

  async getAgentTools(id) {
    return this._c._request("GET", `/api/agents/${id}/tools`);
  }

  async setAgentTools(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/tools`, data);
  }

  async getAgentTraces(id) {
    return this._c._request("GET", `/api/agents/${id}/traces`);
  }

  async updateAgent(id, data) {
    return this._c._request("PUT", `/api/agents/${id}/update`, data);
  }

  async uploadFile(id, data) {
    return this._c._request("POST", `/api/agents/${id}/upload`, data);
  }

  async serveUpload(file_id) {
    return this._c._request("GET", `/api/uploads/${file_id}`);
  }
}

// ── Approvals Resource

class ApprovalsResource {
  constructor(client) { this._c = client; }

  async listApprovals() {
    return this._c._request("GET", "/api/approvals");
  }

  async createApproval(data) {
    return this._c._request("POST", "/api/approvals", data);
  }

  async getApproval(id) {
    return this._c._request("GET", `/api/approvals/${id}`);
  }

  async approveRequest(id, data) {
    return this._c._request("POST", `/api/approvals/${id}/approve`, data);
  }

  async rejectRequest(id) {
    return this._c._request("POST", `/api/approvals/${id}/reject`);
  }
}

// ── Auth Resource

class AuthResource {
  constructor(client) { this._c = client; }

  async authCallback() {
    return this._c._request("GET", "/api/auth/callback");
  }

  async authCallbackPost(data) {
    return this._c._request("POST", "/api/auth/callback", data);
  }

  async authIntrospect(data) {
    return this._c._request("POST", "/api/auth/introspect", data);
  }

  async authLogin() {
    return this._c._request("GET", "/api/auth/login");
  }

  async authLoginProvider(provider) {
    return this._c._request("GET", `/api/auth/login/${provider}`);
  }

  async authProviders() {
    return this._c._request("GET", "/api/auth/providers");
  }

  async authUserinfo() {
    return this._c._request("GET", "/api/auth/userinfo");
  }
}

// ── AutoDream Resource

class AutoDreamResource {
  constructor(client) { this._c = client; }

  async autoDreamAbort(id) {
    return this._c._request("POST", `/api/auto-dream/agents/${id}/abort`);
  }

  async autoDreamSetEnabled(id, data) {
    return this._c._request("PUT", `/api/auto-dream/agents/${id}/enabled`, data);
  }

  async autoDreamTrigger(id) {
    return this._c._request("POST", `/api/auto-dream/agents/${id}/trigger`);
  }

  async autoDreamStatus() {
    return this._c._request("GET", "/api/auto-dream/status");
  }
}

// ── Budget Resource

class BudgetResource {
  constructor(client) { this._c = client; }

  async budgetStatus() {
    return this._c._request("GET", "/api/budget");
  }

  async updateBudget(data) {
    return this._c._request("PUT", "/api/budget", data);
  }

  async agentBudgetRanking() {
    return this._c._request("GET", "/api/budget/agents");
  }

  async agentBudgetStatus(id) {
    return this._c._request("GET", `/api/budget/agents/${id}`);
  }

  async updateAgentBudget(id, data) {
    return this._c._request("PUT", `/api/budget/agents/${id}`, data);
  }

  async usageStats() {
    return this._c._request("GET", "/api/usage");
  }

  async usageByModel() {
    return this._c._request("GET", "/api/usage/by-model");
  }

  async usageDaily() {
    return this._c._request("GET", "/api/usage/daily");
  }

  async usageSummary() {
    return this._c._request("GET", "/api/usage/summary");
  }
}

// ── Channels Resource

class ChannelsResource {
  constructor(client) { this._c = client; }

  async listChannels() {
    return this._c._request("GET", "/api/channels");
  }

  async reloadChannels() {
    return this._c._request("POST", "/api/channels/reload");
  }

  async wechatQrStart() {
    return this._c._request("POST", "/api/channels/wechat/qr/start");
  }

  async wechatQrStatus() {
    return this._c._request("GET", "/api/channels/wechat/qr/status");
  }

  async whatsappQrStart() {
    return this._c._request("POST", "/api/channels/whatsapp/qr/start");
  }

  async whatsappQrStatus() {
    return this._c._request("GET", "/api/channels/whatsapp/qr/status");
  }

  async configureChannel(name, data) {
    return this._c._request("POST", `/api/channels/${name}/configure`, data);
  }

  async removeChannel(name) {
    return this._c._request("DELETE", `/api/channels/${name}/configure`);
  }

  async testChannel(name, data) {
    return this._c._request("POST", `/api/channels/${name}/test`, data);
  }
}

// ── Extensions Resource

class ExtensionsResource {
  constructor(client) { this._c = client; }

  async listExtensions() {
    return this._c._request("GET", "/api/extensions");
  }

  async installExtension(data) {
    return this._c._request("POST", "/api/extensions/install", data);
  }

  async uninstallExtension(data) {
    return this._c._request("POST", "/api/extensions/uninstall", data);
  }

  async getExtension(name) {
    return this._c._request("GET", `/api/extensions/${name}`);
  }
}

// ── Hands Resource

class HandsResource {
  constructor(client) { this._c = client; }

  async listHands() {
    return this._c._request("GET", "/api/hands");
  }

  async listActiveHands() {
    return this._c._request("GET", "/api/hands/active");
  }

  async installHand(data) {
    return this._c._request("POST", "/api/hands/install", data);
  }

  async deactivateHand(id) {
    return this._c._request("DELETE", `/api/hands/instances/${id}`);
  }

  async handInstanceBrowser(id) {
    return this._c._request("GET", `/api/hands/instances/${id}/browser`);
  }

  async pauseHand(id) {
    return this._c._request("POST", `/api/hands/instances/${id}/pause`);
  }

  async resumeHand(id) {
    return this._c._request("POST", `/api/hands/instances/${id}/resume`);
  }

  async handStats(id) {
    return this._c._request("GET", `/api/hands/instances/${id}/stats`);
  }

  async reloadHands() {
    return this._c._request("POST", "/api/hands/reload");
  }

  async getHand(hand_id) {
    return this._c._request("GET", `/api/hands/${hand_id}`);
  }

  async activateHand(hand_id, data) {
    return this._c._request("POST", `/api/hands/${hand_id}/activate`, data);
  }

  async checkHandDeps(hand_id) {
    return this._c._request("POST", `/api/hands/${hand_id}/check-deps`);
  }

  async installHandDeps(hand_id) {
    return this._c._request("POST", `/api/hands/${hand_id}/install-deps`);
  }

  async getHandSettings(hand_id) {
    return this._c._request("GET", `/api/hands/${hand_id}/settings`);
  }

  async updateHandSettings(hand_id, data) {
    return this._c._request("PUT", `/api/hands/${hand_id}/settings`, data);
  }
}

// ── Mcp Resource

class McpResource {
  constructor(client) { this._c = client; }

  async listMcpCatalog() {
    return this._c._request("GET", "/api/mcp/catalog");
  }

  async getMcpCatalogEntry(id) {
    return this._c._request("GET", `/api/mcp/catalog/${id}`);
  }

  async mcpHealthHandler() {
    return this._c._request("GET", "/api/mcp/health");
  }

  async reloadMcpHandler() {
    return this._c._request("POST", "/api/mcp/reload");
  }

  async listMcpServers() {
    return this._c._request("GET", "/api/mcp/servers");
  }

  async addMcpServer(data) {
    return this._c._request("POST", "/api/mcp/servers", data);
  }

  async getMcpServer(name) {
    return this._c._request("GET", `/api/mcp/servers/${name}`);
  }

  async updateMcpServer(name, data) {
    return this._c._request("PUT", `/api/mcp/servers/${name}`, data);
  }

  async deleteMcpServer(name) {
    return this._c._request("DELETE", `/api/mcp/servers/${name}`);
  }

  async reconnectMcpServerHandler(name) {
    return this._c._request("POST", `/api/mcp/servers/${name}/reconnect`);
  }
}

// ── Memory Resource

class MemoryResource {
  constructor(client) { this._c = client; }

  async exportAgentMemory(id) {
    return this._c._request("GET", `/api/agents/${id}/memory/export`);
  }

  async importAgentMemory(id, data) {
    return this._c._request("POST", `/api/agents/${id}/memory/import`, data);
  }

  async getAgentKv(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/kv`);
  }

  async getAgentKvKey(id, key) {
    return this._c._request("GET", `/api/memory/agents/${id}/kv/${key}`);
  }

  async setAgentKvKey(id, key, data) {
    return this._c._request("PUT", `/api/memory/agents/${id}/kv/${key}`, data);
  }

  async deleteAgentKvKey(id, key) {
    return this._c._request("DELETE", `/api/memory/agents/${id}/kv/${key}`);
  }
}

// ── Models Resource

class ModelsResource {
  constructor(client) { this._c = client; }

  async catalogStatus() {
    return this._c._request("GET", "/api/catalog/status");
  }

  async catalogUpdate() {
    return this._c._request("POST", "/api/catalog/update");
  }

  async listModels() {
    return this._c._request("GET", "/api/models");
  }

  async listAliases() {
    return this._c._request("GET", "/api/models/aliases");
  }

  async createAlias(data) {
    return this._c._request("POST", "/api/models/aliases", data);
  }

  async deleteAlias(alias) {
    return this._c._request("DELETE", `/api/models/aliases/${alias}`);
  }

  async addCustomModel(data) {
    return this._c._request("POST", "/api/models/custom", data);
  }

  async removeCustomModel(id) {
    return this._c._request("DELETE", `/api/models/custom/${id}`);
  }

  async getModel(id) {
    return this._c._request("GET", `/api/models/${id}`);
  }

  async listProviders() {
    return this._c._request("GET", "/api/providers");
  }

  async copilotOauthPoll(poll_id) {
    return this._c._request("GET", `/api/providers/github-copilot/oauth/poll/${poll_id}`);
  }

  async copilotOauthStart() {
    return this._c._request("POST", "/api/providers/github-copilot/oauth/start");
  }

  async getProvider(name) {
    return this._c._request("GET", `/api/providers/${name}`);
  }

  async setDefaultProvider(name, data) {
    return this._c._request("POST", `/api/providers/${name}/default`, data);
  }

  async setProviderKey(name, data) {
    return this._c._request("POST", `/api/providers/${name}/key`, data);
  }

  async deleteProviderKey(name) {
    return this._c._request("DELETE", `/api/providers/${name}/key`);
  }

  async testProvider(name) {
    return this._c._request("POST", `/api/providers/${name}/test`);
  }

  async setProviderUrl(name, data) {
    return this._c._request("PUT", `/api/providers/${name}/url`, data);
  }
}

// ── Network Resource

class NetworkResource {
  constructor(client) { this._c = client; }

  async commsEvents() {
    return this._c._request("GET", "/api/comms/events");
  }

  async *commsEventsStream() {
    yield* this._c._stream("GET", "/api/comms/events/stream", undefined);
  }

  async commsSend(data) {
    return this._c._request("POST", "/api/comms/send", data);
  }

  async commsTask(data) {
    return this._c._request("POST", "/api/comms/task", data);
  }

  async commsTopology() {
    return this._c._request("GET", "/api/comms/topology");
  }

  async networkStatus() {
    return this._c._request("GET", "/api/network/status");
  }

  async listPeers() {
    return this._c._request("GET", "/api/peers");
  }

  async getPeer(id) {
    return this._c._request("GET", `/api/peers/${id}`);
  }
}

// ── Pairing Resource

class PairingResource {
  constructor(client) { this._c = client; }

  async pairingComplete(data) {
    return this._c._request("POST", "/api/pairing/complete", data);
  }

  async pairingDevices() {
    return this._c._request("GET", "/api/pairing/devices");
  }

  async pairingRemoveDevice(id) {
    return this._c._request("DELETE", `/api/pairing/devices/${id}`);
  }

  async pairingNotify(data) {
    return this._c._request("POST", "/api/pairing/notify", data);
  }

  async pairingRequest() {
    return this._c._request("POST", "/api/pairing/request");
  }
}

// ── ProactiveMemory Resource

class ProactiveMemoryResource {
  constructor(client) { this._c = client; }

  async memoryList() {
    return this._c._request("GET", "/api/memory");
  }

  async memoryAdd(data) {
    return this._c._request("POST", "/api/memory", data);
  }

  async memoryListAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}`);
  }

  async memoryResetAgent(id) {
    return this._c._request("DELETE", `/api/memory/agents/${id}`);
  }

  async memoryConsolidate(id) {
    return this._c._request("POST", `/api/memory/agents/${id}/consolidate`);
  }

  async memoryDuplicates(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/duplicates`);
  }

  async memoryExportAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/export`);
  }

  async memoryImportAgent(id, data) {
    return this._c._request("POST", `/api/memory/agents/${id}/import`, data);
  }

  async memoryClearLevel(id, level) {
    return this._c._request("DELETE", `/api/memory/agents/${id}/level/${level}`);
  }

  async memorySearchAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/search`);
  }

  async memoryStatsAgent(id) {
    return this._c._request("GET", `/api/memory/agents/${id}/stats`);
  }

  async memoryCleanup() {
    return this._c._request("POST", "/api/memory/cleanup");
  }

  async memoryUpdate(memory_id, data) {
    return this._c._request("PUT", `/api/memory/items/${memory_id}`, data);
  }

  async memoryDelete(memory_id) {
    return this._c._request("DELETE", `/api/memory/items/${memory_id}`);
  }

  async memoryHistory(memory_id) {
    return this._c._request("GET", `/api/memory/items/${memory_id}/history`);
  }

  async memorySearch() {
    return this._c._request("GET", "/api/memory/search");
  }

  async memoryStats() {
    return this._c._request("GET", "/api/memory/stats");
  }

  async memoryGetUser(user_id) {
    return this._c._request("GET", `/api/memory/user/${user_id}`);
  }
}

// ── Sessions Resource

class SessionsResource {
  constructor(client) { this._c = client; }

  async findSessionByLabel(id, label) {
    return this._c._request("GET", `/api/agents/${id}/sessions/by-label/${label}`);
  }

  async listSessions() {
    return this._c._request("GET", "/api/sessions");
  }

  async sessionCleanup() {
    return this._c._request("POST", "/api/sessions/cleanup");
  }

  async getSession(id) {
    return this._c._request("GET", `/api/sessions/${id}`);
  }

  async deleteSession(id) {
    return this._c._request("DELETE", `/api/sessions/${id}`);
  }

  async setSessionLabel(id, data) {
    return this._c._request("PUT", `/api/sessions/${id}/label`, data);
  }
}

// ── Skills Resource

class SkillsResource {
  constructor(client) { this._c = client; }

  async clawhubBrowse() {
    return this._c._request("GET", "/api/clawhub/browse");
  }

  async clawhubInstall(data) {
    return this._c._request("POST", "/api/clawhub/install", data);
  }

  async clawhubSearch() {
    return this._c._request("GET", "/api/clawhub/search");
  }

  async clawhubSkillDetail(slug) {
    return this._c._request("GET", `/api/clawhub/skill/${slug}`);
  }

  async clawhubSkillCode(slug) {
    return this._c._request("GET", `/api/clawhub/skill/${slug}/code`);
  }

  async marketplaceSearch() {
    return this._c._request("GET", "/api/marketplace/search");
  }

  async listSkills() {
    return this._c._request("GET", "/api/skills");
  }

  async createSkill(data) {
    return this._c._request("POST", "/api/skills/create", data);
  }

  async installSkill(data) {
    return this._c._request("POST", "/api/skills/install", data);
  }

  async uninstallSkill(data) {
    return this._c._request("POST", "/api/skills/uninstall", data);
  }

  async listTools() {
    return this._c._request("GET", "/api/tools");
  }

  async getTool(name) {
    return this._c._request("GET", `/api/tools/${name}`);
  }
}

// ── System Resource

class SystemResource {
  constructor(client) { this._c = client; }

  async auditRecent() {
    return this._c._request("GET", "/api/audit/recent");
  }

  async auditVerify() {
    return this._c._request("GET", "/api/audit/verify");
  }

  async createBackup() {
    return this._c._request("POST", "/api/backup");
  }

  async listBackups() {
    return this._c._request("GET", "/api/backups");
  }

  async deleteBackup(filename) {
    return this._c._request("DELETE", `/api/backups/${filename}`);
  }

  async listBindings() {
    return this._c._request("GET", "/api/bindings");
  }

  async addBinding(data) {
    return this._c._request("POST", "/api/bindings", data);
  }

  async removeBinding(index) {
    return this._c._request("DELETE", `/api/bindings/${index}`);
  }

  async listCommands() {
    return this._c._request("GET", "/api/commands");
  }

  async getCommand(name) {
    return this._c._request("GET", `/api/commands/${name}`);
  }

  async getConfig() {
    return this._c._request("GET", "/api/config");
  }

  async configReload() {
    return this._c._request("POST", "/api/config/reload");
  }

  async configSchema() {
    return this._c._request("GET", "/api/config/schema");
  }

  async configSet(data) {
    return this._c._request("POST", "/api/config/set", data);
  }

  async health() {
    return this._c._request("GET", "/api/health");
  }

  async healthDetail() {
    return this._c._request("GET", "/api/health/detail");
  }

  async quickInit() {
    return this._c._request("POST", "/api/init");
  }

  async *logsStream() {
    yield* this._c._stream("GET", "/api/logs/stream", undefined);
  }

  async prometheusMetrics() {
    return this._c._request("GET", "/api/metrics");
  }

  async runMigrate(data) {
    return this._c._request("POST", "/api/migrate", data);
  }

  async migrateDetect() {
    return this._c._request("GET", "/api/migrate/detect");
  }

  async migrateScan(data) {
    return this._c._request("POST", "/api/migrate/scan", data);
  }

  async listProfiles() {
    return this._c._request("GET", "/api/profiles");
  }

  async getProfile(name) {
    return this._c._request("GET", `/api/profiles/${name}`);
  }

  async queueStatus() {
    return this._c._request("GET", "/api/queue/status");
  }

  async restoreBackup(data) {
    return this._c._request("POST", "/api/restore", data);
  }

  async securityStatus() {
    return this._c._request("GET", "/api/security");
  }

  async shutdown() {
    return this._c._request("POST", "/api/shutdown");
  }

  async status() {
    return this._c._request("GET", "/api/status");
  }

  async listAgentTemplates() {
    return this._c._request("GET", "/api/templates");
  }

  async getAgentTemplate(name) {
    return this._c._request("GET", `/api/templates/${name}`);
  }

  async version() {
    return this._c._request("GET", "/api/version");
  }

  async apiVersions() {
    return this._c._request("GET", "/api/versions");
  }
}

// ── Tools Resource

class ToolsResource {
  constructor(client) { this._c = client; }

  async invokeTool(name, data) {
    return this._c._request("POST", `/api/tools/${name}/invoke`, data);
  }
}

// ── Webhooks Resource

class WebhooksResource {
  constructor(client) { this._c = client; }

  async webhookAgent(data) {
    return this._c._request("POST", "/api/hooks/agent", data);
  }

  async webhookWake(data) {
    return this._c._request("POST", "/api/hooks/wake", data);
  }
}

// ── Workflows Resource

class WorkflowsResource {
  constructor(client) { this._c = client; }

  async listCronJobs() {
    return this._c._request("GET", "/api/cron/jobs");
  }

  async createCronJob(data) {
    return this._c._request("POST", "/api/cron/jobs", data);
  }

  async updateCronJob(id, data) {
    return this._c._request("PUT", `/api/cron/jobs/${id}`, data);
  }

  async deleteCronJob(id) {
    return this._c._request("DELETE", `/api/cron/jobs/${id}`);
  }

  async toggleCronJob(id, data) {
    return this._c._request("PUT", `/api/cron/jobs/${id}/enable`, data);
  }

  async cronJobStatus(id) {
    return this._c._request("GET", `/api/cron/jobs/${id}/status`);
  }

  async listSchedules() {
    return this._c._request("GET", "/api/schedules");
  }

  async createSchedule(data) {
    return this._c._request("POST", "/api/schedules", data);
  }

  async getSchedule(id) {
    return this._c._request("GET", `/api/schedules/${id}`);
  }

  async updateSchedule(id, data) {
    return this._c._request("PUT", `/api/schedules/${id}`, data);
  }

  async deleteSchedule(id) {
    return this._c._request("DELETE", `/api/schedules/${id}`);
  }

  async runSchedule(id) {
    return this._c._request("POST", `/api/schedules/${id}/run`);
  }

  async listTriggers() {
    return this._c._request("GET", "/api/triggers");
  }

  async createTrigger(data) {
    return this._c._request("POST", "/api/triggers", data);
  }

  async getTrigger(id) {
    return this._c._request("GET", `/api/triggers/${id}`);
  }

  async deleteTrigger(id) {
    return this._c._request("DELETE", `/api/triggers/${id}`);
  }

  async updateTrigger(id, data) {
    return this._c._request("PATCH", `/api/triggers/${id}`, data);
  }

  async listWorkflows() {
    return this._c._request("GET", "/api/workflows");
  }

  async createWorkflow(data) {
    return this._c._request("POST", "/api/workflows", data);
  }

  async updateWorkflow(id, data) {
    return this._c._request("PUT", `/api/workflows/${id}`, data);
  }

  async deleteWorkflow(id) {
    return this._c._request("DELETE", `/api/workflows/${id}`);
  }

  async runWorkflow(id, data) {
    return this._c._request("POST", `/api/workflows/${id}/run`, data);
  }

  async listWorkflowRuns(id) {
    return this._c._request("GET", `/api/workflows/${id}/runs`);
  }

  async saveWorkflowAsTemplate(id) {
    return this._c._request("POST", `/api/workflows/${id}/save-as-template`);
  }
}

module.exports = { LibreFang, LibreFangError };
