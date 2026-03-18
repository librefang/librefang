// LibreFang Settings Page — Provider Hub, Model Catalog, Config, Tools + Security, Network, Migration tabs
'use strict';

function settingsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    tab: 'providers',
    sysInfo: {},
    usageData: [],
    tools: [],
    config: {},
    providers: [],
    models: [],
    toolSearch: '',
    modelSearch: '',
    modelProviderFilter: '',
    modelTierFilter: '',
    showCustomModelForm: false,
    customModelId: '',
    customModelProvider: 'openrouter',
    customModelContext: 128000,
    customModelMaxOutput: 8192,
    customModelStatus: '',
    providerKeyInputs: {},
    providerUrlInputs: {},
    providerUrlSaving: {},
    providerTesting: {},
    providerTestResults: {},
    copilotOAuth: { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 },
    customProviderName: '',
    customProviderUrl: '',
    customProviderKey: '',
    customProviderStatus: '',
    addingCustomProvider: false,
    catalogStatus: { last_sync: null },
    catalogUpdating: false,
    catalogResult: '',
    loading: true,
    loadError: '',

    _updateURL() {
      var params = [];
      if (this.tab && this.tab !== 'providers') params.push('tab=' + encodeURIComponent(this.tab));
      var hash = 'settings' + (params.length ? '?' + params.join('&') : '');
      if (window.location.hash !== '#' + hash) history.replaceState(null, '', '#' + hash);
    },

    init() {
      var self = this;
      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
      });
      var hashParts = window.location.hash.split('?');
      if (hashParts.length > 1) {
        var params = new URLSearchParams(hashParts[1]);
        if (params.get('tab')) self.tab = params.get('tab');
      }
      this.$watch('tab', function() { self._updateURL(); });
    },

    interpolate(text, params) {
      if (!params || typeof text !== 'string') return text;
      return text.replace(/\{(\w+)\}/g, function(match, key) {
        return params[key] !== undefined ? params[key] : match;
      });
    },

    t(key, fallback, params) {
      if (typeof i18n === 'undefined') return this.interpolate(fallback || key, params);
      var translated = i18n.t(key, params);
      if (!translated || translated.charAt(0) === '[') {
        return this.interpolate(fallback || key, params);
      }
      return translated;
    },

    providerModelsText(provider) {
      return this.t('settingsPage.providerModels', '{count} model(s) available', {
        count: provider.model_count || 0
      });
    },

    envText(env) {
      return this.t('settingsPage.envLabel', 'Env: {env}', { env: env });
    },

    providerKeyPlaceholder(env) {
      return this.t('settingsPage.enterEnv', 'Enter {env}', { env: env });
    },

    envRestartText(env) {
      return this.t('settingsPage.envRestart', 'Or set {env} in your environment and restart', {
        env: env
      });
    },

    copilotVisitText() {
      return this.t('settingsPage.visitAndEnter', 'Visit {url} and enter:', {
        url: this.copilotOAuth.verificationUri || ''
      });
    },

    customModelToggleText() {
      return this.showCustomModelForm
        ? this.t('btn.cancel', 'Cancel')
        : this.t('settingsPage.customModelButton', '+ Custom Model');
    },

    modelsSummaryText() {
      return this.t('settingsPage.modelsSummary', '{filtered} of {total} models', {
        filtered: this.filteredModels.length,
        total: this.models.length
      });
    },

    modelEmptyTitle() {
      if (this.models.length) return this.t('settingsPage.noModelsMatch', 'No models match your search');
      return this.t('settingsPage.noModelsAvailable', 'No models available');
    },

    modelEmptyDesc() {
      if (this.models.length) return this.t('settingsPage.noModelsMatchDesc', 'Try a different search term or clear filters.');
      return this.t('settingsPage.noModelsAvailableDesc', 'Configure an LLM provider to see available models.');
    },

    modelStatusText(model) {
      return model.available
        ? this.t('settingsPage.available', 'Available')
        : this.t('settingsPage.needsKey', 'Needs Key');
    },

    toolsSummaryText() {
      return this.t('settingsPage.toolsSummary', '{filtered} of {total} tools', {
        filtered: this.filteredTools.length,
        total: this.tools.length
      });
    },

    toolsEmptyTitle() {
      if (this.tools.length) return this.t('settingsPage.noToolsMatch', 'No tools match your search');
      return this.t('settingsPage.noToolsAvailable', 'No tools available');
    },

    toolsEmptyDesc() {
      if (this.tools.length) return this.t('settingsPage.noToolsMatchDesc', 'Try a different search term.');
      return this.t('settingsPage.noToolsAvailableDesc', 'Tools will appear once agents are configured.');
    },

    providerNamePlaceholder() {
      return this.t('settingsPage.providerNamePlaceholder', 'e.g. my-local-llm');
    },

    providerUrlPlaceholder() {
      return this.t('settingsPage.providerUrlPlaceholder', 'http://localhost:8080/v1');
    },

    providerKeyOptionalPlaceholder() {
      return this.t('settingsPage.providerKeyOptionalPlaceholder', 'sk-... (leave blank if not needed)');
    },

    customModelIdPlaceholder() {
      return this.t('settingsPage.customModelIdPlaceholder', 'e.g. my-org/my-model');
    },

    customModelProviderPlaceholder() {
      return this.t('settingsPage.customModelProviderPlaceholder', 'openrouter');
    },

    chainResultText() {
      if (!this.chainResult) return '';
      if (this.chainResult.valid) {
        return this.t('settingsPage.chainValid', 'CHAIN VALID - {count} entries verified', {
          count: this.chainResult.entries || 0
        });
      }
      return this.t('settingsPage.chainBroken', 'CHAIN BROKEN - {message}', {
        message: this.chainResult.error || ''
      });
    },

    securityStateText(enabled) {
      return enabled
        ? this.t('settingsPage.active', 'Active')
        : this.t('settingsPage.disabled', 'Disabled');
    },

    monitoringAvailableText(available) {
      return available
        ? this.t('settingsPage.available', 'Available')
        : this.t('settingsPage.notAvailable', 'Not available');
    },

    featureText(group, feature, field, fallback) {
      return this.t('settingsPage.' + group + '.' + feature.key + '.' + field, fallback);
    },

    coreFeatureName(feature) {
      return this.featureText('coreFeatures', feature, 'name', feature.name);
    },

    coreFeatureDescription(feature) {
      return this.featureText('coreFeatures', feature, 'description', feature.description);
    },

    coreFeatureThreat(feature) {
      return this.featureText('coreFeatures', feature, 'threat', feature.threat);
    },

    configurableFeatureName(feature) {
      return this.featureText('configurableFeatures', feature, 'name', feature.name);
    },

    configurableFeatureDescription(feature) {
      return this.featureText('configurableFeatures', feature, 'description', feature.description);
    },

    configurableFeatureHint(feature) {
      return this.featureText('configurableFeatures', feature, 'hint', feature.configHint);
    },

    monitoringFeatureName(feature) {
      return this.featureText('monitoringFeatures', feature, 'name', feature.name);
    },

    monitoringFeatureDescription(feature) {
      return this.featureText('monitoringFeatures', feature, 'description', feature.description);
    },

    monitoringFeatureHint(feature) {
      return this.featureText('monitoringFeatures', feature, 'hint', feature.configHint);
    },

    peerStateText(state) {
      return this.t('settingsPage.peerState.' + String(state || '').toLowerCase(), state);
    },

    // -- Proactive Memory state --
    pmSettings: {
      auto_memorize: true,
      auto_retrieve: true,
      max_retrieve: 5,
      extraction_threshold: 0.7,
      extraction_model: '',
      max_memories_per_agent: 1000,
      extract_categories: [],
      session_ttl_hours: 24,
      confidence_decay_rate: 0.01,
      duplicate_threshold: 0.5,
    },
    pmLoaded: false,
    pmLoading: false,
    pmSaving: false,
    pmSaveStatus: '',
    pmCategoriesText: '',

    // -- Dynamic config state --
    configSchema: null,
    configValues: {},
    configDirty: {},
    configSaving: {},

    // -- Security state --
    securityData: null,
    secLoading: false,
    verifyingChain: false,
    chainResult: null,

    coreFeatures: [
      {
        name: 'Path Traversal Prevention', key: 'path_traversal',
        description: 'Blocks directory escape attacks (../) in all file operations. Two-phase validation: syntactic rejection of path components, then canonicalization to normalize symlinks.',
        threat: 'Directory escape, privilege escalation via symlinks',
        impl: 'host_functions.rs — safe_resolve_path() + safe_resolve_parent()'
      },
      {
        name: 'SSRF Protection', key: 'ssrf_protection',
        description: 'Blocks outbound requests to private IPs, localhost, and cloud metadata endpoints (AWS/GCP/Azure). Validates DNS resolution results to defeat rebinding attacks.',
        threat: 'Internal network reconnaissance, cloud credential theft',
        impl: 'host_functions.rs — is_ssrf_target() + is_private_ip()'
      },
      {
        name: 'Capability-Based Access Control', key: 'capability_system',
        description: 'Deny-by-default permission system. Every agent operation (file I/O, network, shell, memory, spawn) requires an explicit capability grant in the manifest.',
        threat: 'Unauthorized resource access, sandbox escape',
        impl: 'host_functions.rs — check_capability() on every host function'
      },
      {
        name: 'Privilege Escalation Prevention', key: 'privilege_escalation_prevention',
        description: 'When a parent agent spawns a child, the kernel enforces child capabilities are a subset of parent capabilities. No agent can grant rights it does not have.',
        threat: 'Capability escalation through agent spawning chains',
        impl: 'kernel_handle.rs — spawn_agent_checked()'
      },
      {
        name: 'Subprocess Environment Isolation', key: 'subprocess_isolation',
        description: 'Child processes (shell tools) inherit only a safe allow-list of environment variables. API keys, database passwords, and secrets are never leaked to subprocesses.',
        threat: 'Secret exfiltration via child process environment',
        impl: 'subprocess_sandbox.rs — env_clear() + SAFE_ENV_VARS'
      },
      {
        name: 'Security Headers', key: 'security_headers',
        description: 'Every HTTP response includes CSP, X-Frame-Options: DENY, X-Content-Type-Options: nosniff, Referrer-Policy, and X-XSS-Protection headers.',
        threat: 'XSS, clickjacking, MIME sniffing, content injection',
        impl: 'middleware.rs — security_headers()'
      },
      {
        name: 'Wire Protocol Authentication', key: 'wire_hmac_auth',
        description: 'Agent-to-agent OFP connections use HMAC-SHA256 mutual authentication with nonce-based handshake and constant-time signature comparison (subtle crate).',
        threat: 'Man-in-the-middle attacks on mesh network',
        impl: 'peer.rs — hmac_sign() + hmac_verify()'
      },
      {
        name: 'Request ID Tracking', key: 'request_id_tracking',
        description: 'Every API request receives a unique UUID (x-request-id header) and is logged with method, path, status code, and latency for full traceability.',
        threat: 'Untraceable actions, forensic blind spots',
        impl: 'middleware.rs — request_logging()'
      }
    ],

    configurableFeatures: [
      {
        name: 'API Rate Limiting', key: 'rate_limiter',
        description: 'GCRA (Generic Cell Rate Algorithm) with cost-aware tokens. Different endpoints cost different amounts — spawning an agent costs 50 tokens, health check costs 1.',
        configHint: 'Hard-coded: 500 tokens/minute per IP. Edit rate_limiter.rs to tune.',
        valueKey: 'rate_limiter'
      },
      {
        name: 'WebSocket Connection Limits', key: 'websocket_limits',
        description: 'Per-IP connection cap prevents connection exhaustion. Idle timeout closes abandoned connections. Message rate limiting prevents flooding.',
        configHint: 'Hard-coded: 5 connections/IP, 30min idle timeout, 64KB max message. Edit ws.rs to tune.',
        valueKey: 'websocket_limits'
      },
      {
        name: 'WASM Dual Metering', key: 'wasm_sandbox',
        description: 'WASM modules run with two independent resource limits: fuel metering (CPU instruction count) and epoch interruption (wall-clock timeout with watchdog thread).',
        configHint: 'Default: 1M fuel units, 30s timeout. Configurable per-agent via SandboxConfig.',
        valueKey: 'wasm_sandbox'
      },
      {
        name: 'Bearer Token Authentication', key: 'auth',
        description: 'All non-health endpoints require Authorization: Bearer header. When no API key is configured, all requests are restricted to localhost only.',
        configHint: 'Set api_key in ~/.librefang/config.toml for remote access. Empty = localhost only.',
        valueKey: 'auth'
      }
    ],

    monitoringFeatures: [
      {
        name: 'Merkle Audit Trail', key: 'audit_trail',
        description: 'Every security-critical action is appended to an immutable, tamper-evident log. Each entry is cryptographically linked to the previous via SHA-256 hash chain.',
        configHint: 'Always active. Verify chain integrity from the Audit Log page.',
        valueKey: 'audit_trail'
      },
      {
        name: 'Information Flow Taint Tracking', key: 'taint_tracking',
        description: 'Labels data by provenance (ExternalNetwork, UserInput, PII, Secret, UntrustedAgent) and blocks unsafe flows: external data cannot reach shell_exec, secrets cannot reach network.',
        configHint: 'Always active. Prevents data flow attacks automatically.',
        valueKey: 'taint_tracking'
      },
      {
        name: 'Ed25519 Manifest Signing', key: 'manifest_signing',
        description: 'Agent manifests can be cryptographically signed with Ed25519. Verify manifest integrity before loading to prevent supply chain tampering.',
        configHint: 'Available for use. Sign manifests with ed25519-dalek for verification.',
        valueKey: 'manifest_signing'
      }
    ],

    // -- Peers state --
    peers: [],
    peersLoading: false,
    peersLoadError: '',
    _peerPollTimer: null,

    // -- Migration state --
    migStep: 'intro',
    detecting: false,
    scanning: false,
    migrating: false,
    sourcePath: '',
    targetPath: '',
    scanResult: null,
    migResult: null,

    // -- Settings load --
    async loadSettings() {
      this.loading = true;
      this.loadError = '';
      try {
        await Promise.all([
          this.loadSysInfo(),
          this.loadUsage(),
          this.loadTools(),
          this.loadConfig(),
          this.loadProviders(),
          this.loadModels(),
          this.fetchCatalogStatus()
        ]);
      } catch(e) {
        this.loadError = e.message || this.t('settingsPage.loadError', 'Could not load settings.');
      }
      this.loading = false;
    },

    async loadData() { return this.loadSettings(); },

    async loadSysInfo() {
      try {
        var ver = await LibreFangAPI.get('/api/version');
        var status = await LibreFangAPI.get('/api/status');
        this.sysInfo = {
          version: ver.version || '-',
          platform: ver.platform || '-',
          arch: ver.arch || '-',
          uptime_seconds: status.uptime_seconds || 0,
          agent_count: status.agent_count || 0,
          default_provider: status.default_provider || '-',
          default_model: status.default_model || '-'
        };
      } catch(e) { throw e; }
    },

    async loadUsage() {
      try {
        var data = await LibreFangAPI.get('/api/usage');
        this.usageData = data.agents || [];
      } catch(e) { this.usageData = []; }
    },

    async loadTools() {
      try {
        var data = await LibreFangAPI.get('/api/tools');
        this.tools = data.tools || [];
      } catch(e) { this.tools = []; }
    },

    async loadConfig() {
      try {
        this.config = await LibreFangAPI.get('/api/config');
      } catch(e) { this.config = {}; }
    },

    async loadProviders() {
      try {
        var data = await LibreFangAPI.get('/api/providers');
        this.providers = (data.providers || []).sort(function(a, b) {
          return (a.auth_status === 'configured' ? 0 : 1) - (b.auth_status === 'configured' ? 0 : 1);
        });
        for (var i = 0; i < this.providers.length; i++) {
          var p = this.providers[i];
          if (p.is_local) {
            if (!this.providerUrlInputs[p.id]) {
              this.providerUrlInputs[p.id] = p.base_url || '';
            }
            if (this.providerUrlSaving[p.id] === undefined) {
              this.providerUrlSaving[p.id] = false;
            }
          }
        }
      } catch(e) { this.providers = []; }
    },

    async loadModels() {
      try {
        var data = await LibreFangAPI.get('/api/models');
        this.models = data.models || [];
      } catch(e) { this.models = []; }
    },

    async addCustomModel() {
      var id = this.customModelId.trim();
      if (!id) return;
      this.customModelStatus = this.t('settingsPage.adding', 'Adding...');
      try {
        await LibreFangAPI.post('/api/models/custom', {
          id: id,
          provider: this.customModelProvider || 'openrouter',
          context_window: this.customModelContext || 128000,
          max_output_tokens: this.customModelMaxOutput || 8192,
        });
        this.customModelStatus = this.t('settingsPage.added', 'Added!');
        this.customModelId = '';
        this.showCustomModelForm = false;
        await this.loadModels();
      } catch(e) {
        this.customModelStatus = this.t('settingsPage.errorMessage', 'Error: {message}', {
          message: e.message || this.t('settingsPage.failed', 'Failed')
        });
      }
    },

    async deleteCustomModel(modelId) {
      if (!confirm(this.t('settingsPage.deleteCustomModelConfirm', 'Delete custom model "{model}"?', { model: modelId }))) return;
      try {
        await LibreFangAPI.del('/api/models/custom/' + encodeURIComponent(modelId));
        LibreFangToast.success(this.t('settingsPage.modelDeleted', 'Model deleted'));
        await this.loadModels();
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.deleteFailed', 'Failed to delete: {message}', {
          message: e.message || this.t('settingsPage.unknownError', 'Unknown error')
        }));
      }
    },

    async loadConfigSchema() {
      try {
        var results = await Promise.all([
          LibreFangAPI.get('/api/config/schema').catch(function() { return {}; }),
          LibreFangAPI.get('/api/config')
        ]);
        this.configSchema = results[0].sections || null;
        this.configValues = results[1] || {};
      } catch(e) { /* silent */ }
    },

    isConfigDirty(section, field) {
      return this.configDirty[section + '.' + field] === true;
    },

    markConfigDirty(section, field) {
      this.configDirty[section + '.' + field] = true;
    },

    async saveConfigField(section, field, value) {
      var key = section + '.' + field;
      // Root-level fields (api_key, api_listen, log_level) use just the field name
      var sectionMeta = this.configSchema && this.configSchema[section];
      var path = (sectionMeta && sectionMeta.root_level) ? field : key;
      this.configSaving[key] = true;
      try {
        await LibreFangAPI.post('/api/config/set', { path: path, value: value });
        this.configDirty[key] = false;
        LibreFangToast.success(this.t('settingsPage.fieldSaved', 'Saved {field}', { field: field }));
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.saveFailed', 'Failed to save: {message}', { message: e.message }));
      }
      this.configSaving[key] = false;
    },

    get filteredTools() {
      var q = this.toolSearch.toLowerCase().trim();
      if (!q) return this.tools;
      return this.tools.filter(function(t) {
        return t.name.toLowerCase().indexOf(q) !== -1 ||
               (t.description || '').toLowerCase().indexOf(q) !== -1;
      });
    },

    get filteredModels() {
      var self = this;
      return this.models.filter(function(m) {
        if (self.modelProviderFilter && m.provider !== self.modelProviderFilter) return false;
        if (self.modelTierFilter && m.tier !== self.modelTierFilter) return false;
        if (self.modelSearch) {
          var q = self.modelSearch.toLowerCase();
          if (m.id.toLowerCase().indexOf(q) === -1 &&
              (m.display_name || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    get uniqueProviderNames() {
      var seen = {};
      this.models.forEach(function(m) { seen[m.provider] = true; });
      return Object.keys(seen).sort();
    },

    get uniqueTiers() {
      var seen = {};
      this.models.forEach(function(m) { if (m.tier) seen[m.tier] = true; });
      return Object.keys(seen).sort();
    },

    providerAuthClass(p) {
      if (p.auth_status === 'configured') return 'auth-configured';
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') return 'auth-not-set';
      return 'auth-no-key';
    },

    providerAuthText(p) {
      if (p.auth_status === 'configured') return this.t('settingsPage.configured', 'Configured');
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') {
        if (p.id === 'claude-code') return this.t('settingsPage.notInstalled', 'Not Installed');
        return this.t('settingsPage.notSet', 'Not Set');
      }
      return this.t('settingsPage.noKeyNeeded', 'No Key Needed');
    },

    providerCardClass(p) {
      if (p.auth_status === 'configured') return 'configured';
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') return 'not-configured';
      return 'no-key';
    },

    tierBadgeClass(tier) {
      if (!tier) return '';
      var t = tier.toLowerCase();
      if (t === 'frontier') return 'tier-frontier';
      if (t === 'smart') return 'tier-smart';
      if (t === 'balanced') return 'tier-balanced';
      if (t === 'fast') return 'tier-fast';
      return '';
    },

    formatCost(cost) {
      if (!cost && cost !== 0) return '-';
      return '$' + cost.toFixed(4);
    },

    formatContext(ctx) {
      if (!ctx) return '-';
      if (ctx >= 1000000) return (ctx / 1000000).toFixed(1) + 'M';
      if (ctx >= 1000) return Math.round(ctx / 1000) + 'K';
      return String(ctx);
    },

    formatUptime(secs) {
      if (!secs) return '-';
      var h = Math.floor(secs / 3600);
      var m = Math.floor((secs % 3600) / 60);
      var s = secs % 60;
      if (h > 0) return this.t('runtimePage.hoursMinutesShort', '{hours}h {minutes}m', { hours: h, minutes: m });
      if (m > 0) return this.t('runtimePage.minutesSecondsShort', '{minutes}m {seconds}s', { minutes: m, seconds: s });
      return this.t('runtimePage.secondsShort', '{count}s', { count: s });
    },

    async saveProviderKey(provider) {
      var key = this.providerKeyInputs[provider.id];
      if (!key || !key.trim()) { LibreFangToast.error(this.t('settingsPage.enterApiKey', 'Please enter an API key')); return; }
      try {
        await LibreFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/key', { key: key.trim() });
        LibreFangToast.success(this.t('settingsPage.apiKeySaved', 'API key saved for {provider}', { provider: provider.display_name }));
        this.providerKeyInputs[provider.id] = '';
        await this.loadProviders();
        await this.loadModels();
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.saveKeyFailed', 'Failed to save key: {message}', { message: e.message }));
      }
    },

    removeProviderKey(provider) {
      var self = this;
      LibreFangToast.confirm(
        this.t('settingsPage.removeKey', 'Remove Key'),
        this.t('settingsPage.confirmRemoveKey', 'Are you sure you want to remove the API key for {provider}?', { provider: provider.display_name }),
        async function() {
          try {
            await LibreFangAPI.del('/api/providers/' + encodeURIComponent(provider.id) + '/key');
            LibreFangToast.success(self.t('settingsPage.apiKeyRemoved', 'API key removed for {provider}', { provider: provider.display_name }));
            await self.loadProviders();
            await self.loadModels();
          } catch(e) {
            LibreFangToast.error(self.t('settingsPage.removeKeyFailed', 'Failed to remove key: {message}', { message: e.message }));
          }
        }
      );
    },

    async startCopilotOAuth() {
      this.copilotOAuth.polling = true;
      this.copilotOAuth.userCode = '';
      try {
        var resp = await LibreFangAPI.post('/api/providers/github-copilot/oauth/start', {});
        this.copilotOAuth.userCode = resp.user_code;
        this.copilotOAuth.verificationUri = resp.verification_uri;
        this.copilotOAuth.pollId = resp.poll_id;
        this.copilotOAuth.interval = resp.interval || 5;
        window.open(resp.verification_uri, '_blank');
        this.pollCopilotOAuth();
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.startCopilotFailed', 'Failed to start Copilot login: {message}', { message: e.message }));
        this.copilotOAuth.polling = false;
      }
    },

    pollCopilotOAuth() {
      var self = this;
      setTimeout(async function() {
        if (!self.copilotOAuth.pollId) return;
        try {
          var resp = await LibreFangAPI.get('/api/providers/github-copilot/oauth/poll/' + self.copilotOAuth.pollId);
          if (resp.status === 'complete') {
            LibreFangToast.success(self.t('settingsPage.copilotAuthenticated', 'GitHub Copilot authenticated successfully!'));
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
            await self.loadProviders();
            await self.loadModels();
          } else if (resp.status === 'pending') {
            if (resp.interval) self.copilotOAuth.interval = resp.interval;
            self.pollCopilotOAuth();
          } else if (resp.status === 'expired') {
            LibreFangToast.error(self.t('settingsPage.deviceCodeExpired', 'Device code expired. Please try again.'));
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          } else if (resp.status === 'denied') {
            LibreFangToast.error(self.t('settingsPage.accessDenied', 'Access denied by user.'));
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          } else {
            LibreFangToast.error(self.t('settingsPage.oauthError', 'OAuth error: {message}', { message: resp.error || resp.status }));
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          }
        } catch(e) {
          LibreFangToast.error(self.t('settingsPage.pollError', 'Poll error: {message}', { message: e.message }));
          self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
        }
      }, self.copilotOAuth.interval * 1000);
    },

    async testProvider(provider) {
      this.providerTesting[provider.id] = true;
      this.providerTestResults[provider.id] = null;
      try {
        var result = await LibreFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/test', {});
        this.providerTestResults[provider.id] = result;
        if (result.status === 'ok') {
          LibreFangToast.success(this.t('settingsPage.providerConnected', '{provider} connected ({latency}ms)', {
            provider: provider.display_name,
            latency: result.latency_ms || '?'
          }));
        } else {
          LibreFangToast.error(this.t('settingsPage.providerConnectionFailed', '{provider}: {message}', {
            provider: provider.display_name,
            message: result.error || this.t('settingsPage.connectionFailed', 'Connection failed')
          }));
        }
      } catch(e) {
        this.providerTestResults[provider.id] = { status: 'error', error: e.message };
        LibreFangToast.error(this.t('settingsPage.testFailed', 'Test failed: {message}', { message: e.message }));
      }
      this.providerTesting[provider.id] = false;
    },

    async saveProviderUrl(provider) {
      var url = this.providerUrlInputs[provider.id];
      if (!url || !url.trim()) { LibreFangToast.error(this.t('settingsPage.enterBaseUrl', 'Please enter a base URL')); return; }
      url = url.trim();
      if (url.indexOf('http://') !== 0 && url.indexOf('https://') !== 0) {
        LibreFangToast.error(this.t('settingsPage.urlMustStart', 'URL must start with http:// or https://')); return;
      }
      this.providerUrlSaving[provider.id] = true;
      try {
        var result = await LibreFangAPI.put('/api/providers/' + encodeURIComponent(provider.id) + '/url', { base_url: url });
        if (result.reachable) {
          LibreFangToast.success(this.t('settingsPage.urlSavedReachable', '{provider} URL saved - reachable ({latency}ms)', {
            provider: provider.display_name,
            latency: result.latency_ms || '?'
          }));
        } else {
          LibreFangToast.warning(this.t('settingsPage.urlSavedNotReachable', '{provider} URL saved but not reachable', {
            provider: provider.display_name
          }));
        }
        await this.loadProviders();
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.saveUrlFailed', 'Failed to save URL: {message}', { message: e.message }));
      }
      this.providerUrlSaving[provider.id] = false;
    },

    async addCustomProvider() {
      var name = this.customProviderName.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-').replace(/-+/g, '-');
      if (!name) { LibreFangToast.error(this.t('settingsPage.enterProviderName', 'Please enter a provider name')); return; }
      var url = this.customProviderUrl.trim();
      if (!url) { LibreFangToast.error(this.t('settingsPage.enterBaseUrl', 'Please enter a base URL')); return; }
      if (url.indexOf('http://') !== 0 && url.indexOf('https://') !== 0) {
        LibreFangToast.error(this.t('settingsPage.urlMustStart', 'URL must start with http:// or https://')); return;
      }
      this.addingCustomProvider = true;
      this.customProviderStatus = '';
      try {
        var result = await LibreFangAPI.put('/api/providers/' + encodeURIComponent(name) + '/url', { base_url: url });
        if (this.customProviderKey.trim()) {
          await LibreFangAPI.post('/api/providers/' + encodeURIComponent(name) + '/key', { key: this.customProviderKey.trim() });
        }
        this.customProviderName = '';
        this.customProviderUrl = '';
        this.customProviderKey = '';
        this.customProviderStatus = '';
        LibreFangToast.success(this.t(
          result.reachable ? 'settingsPage.providerAddedReachable' : 'settingsPage.providerAddedNotReachable',
          result.reachable ? 'Provider "{name}" added (reachable)' : 'Provider "{name}" added (not reachable yet)',
          { name: name }
        ));
        await this.loadProviders();
      } catch(e) {
        this.customProviderStatus = this.t('settingsPage.errorMessage', 'Error: {message}', {
          message: e.message || this.t('settingsPage.failed', 'Failed')
        });
        LibreFangToast.error(this.t('settingsPage.addProviderFailed', 'Failed to add provider: {message}', { message: e.message }));
      }
      this.addingCustomProvider = false;
    },

    // -- Proactive Memory methods --
    async loadProactiveMemory() {
      this.pmLoading = true;
      try {
        var data = await LibreFangAPI.get('/api/config');
        var pm = data.proactive_memory || {};
        this.pmSettings = {
          auto_memorize: pm.auto_memorize !== undefined ? pm.auto_memorize : true,
          auto_retrieve: pm.auto_retrieve !== undefined ? pm.auto_retrieve : true,
          max_retrieve: pm.max_retrieve || 5,
          extraction_threshold: pm.extraction_threshold !== undefined ? pm.extraction_threshold : 0.7,
          extraction_model: pm.extraction_model || '',
          extract_categories: pm.extract_categories || [],
          session_ttl_hours: pm.session_ttl_hours || 24,
          confidence_decay_rate: pm.confidence_decay_rate !== undefined ? pm.confidence_decay_rate : 0.01,
          duplicate_threshold: pm.duplicate_threshold !== undefined ? pm.duplicate_threshold : 0.5,
          max_memories_per_agent: pm.max_memories_per_agent !== undefined ? pm.max_memories_per_agent : 1000,
        };
        this.pmCategoriesText = (this.pmSettings.extract_categories || []).join(', ');
        this.pmLoaded = true;
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.loadProactiveMemoryFailed', 'Failed to load proactive memory settings: {message}', { message: e.message }));
      }
      this.pmLoading = false;
    },

    async saveProactiveMemory() {
      this.pmSaving = true;
      this.pmSaveStatus = '';
      try {
        var mr = Number(this.pmSettings.max_retrieve);
        var et = Number(this.pmSettings.extraction_threshold);
        var dt = Number(this.pmSettings.duplicate_threshold);
        var cd = Number(this.pmSettings.confidence_decay_rate);
        var st = Number(this.pmSettings.session_ttl_hours);
        var mma = Number(this.pmSettings.max_memories_per_agent);
        if (isNaN(mr) || mr < 1 || mr > 100) { LibreFangToast.error('max_retrieve must be 1–100'); this.pmSaving = false; return; }
        if (isNaN(et) || et < 0 || et > 1) { LibreFangToast.error('extraction_threshold must be 0–1'); this.pmSaving = false; return; }
        if (isNaN(dt) || dt < 0 || dt > 1) { LibreFangToast.error('duplicate_threshold must be 0–1'); this.pmSaving = false; return; }
        if (isNaN(cd) || cd < 0 || cd > 1) { LibreFangToast.error('confidence_decay_rate must be 0–1'); this.pmSaving = false; return; }
        if (isNaN(st) || st < 1) { LibreFangToast.error('session_ttl_hours must be >= 1'); this.pmSaving = false; return; }
        if (isNaN(mma) || mma < 0) { LibreFangToast.error('max_memories_per_agent must be >= 0'); this.pmSaving = false; return; }
        var fields = [
          { path: 'proactive_memory.auto_memorize', value: this.pmSettings.auto_memorize },
          { path: 'proactive_memory.auto_retrieve', value: this.pmSettings.auto_retrieve },
          { path: 'proactive_memory.max_retrieve', value: mr },
          { path: 'proactive_memory.extraction_threshold', value: et },
          { path: 'proactive_memory.session_ttl_hours', value: st },
          { path: 'proactive_memory.confidence_decay_rate', value: cd },
          { path: 'proactive_memory.duplicate_threshold', value: dt },
          { path: 'proactive_memory.max_memories_per_agent', value: mma },
        ];
        // Only set extraction_model if non-empty, otherwise set to empty string to clear it
        fields.push({ path: 'proactive_memory.extraction_model', value: this.pmSettings.extraction_model || '' });
        // Save categories as comma-separated string to TOML array
        if (this.pmSettings.extract_categories && this.pmSettings.extract_categories.length > 0) {
          fields.push({ path: 'proactive_memory.extract_categories', value: this.pmSettings.extract_categories });
        }

        for (var i = 0; i < fields.length; i++) {
          await LibreFangAPI.post('/api/config/set', { path: fields[i].path, value: fields[i].value });
        }
        this.pmSaveStatus = '';
        LibreFangToast.success(this.t('settingsPage.proactiveMemorySaved', 'Proactive memory settings saved'));
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.saveFailed', 'Failed to save: {message}', { message: e.message }));
      }
      this.pmSaving = false;
    },

    // -- Security methods --
    async loadSecurity() {
      this.secLoading = true;
      try {
        this.securityData = await LibreFangAPI.get('/api/security');
      } catch(e) {
        this.securityData = null;
      }
      this.secLoading = false;
    },

    isActive(key) {
      if (!this.securityData) return true;
      var core = this.securityData.core_protections || {};
      if (core[key] !== undefined) return core[key];
      return true;
    },

    getConfigValue(key) {
      if (!this.securityData) return null;
      var cfg = this.securityData.configurable || {};
      return cfg[key] || null;
    },

    getMonitoringValue(key) {
      if (!this.securityData) return null;
      var mon = this.securityData.monitoring || {};
      return mon[key] || null;
    },

    formatConfigValue(feature) {
      var val = this.getConfigValue(feature.valueKey);
      if (!val) return this.configurableFeatureHint(feature);
      switch (feature.valueKey) {
        case 'rate_limiter':
          return this.t('settingsPage.rateLimiterValue', 'Algorithm: {algorithm} | {count} tokens/min per IP', {
            algorithm: val.algorithm || 'GCRA',
            count: val.tokens_per_minute || 500
          });
        case 'websocket_limits':
          return this.t('settingsPage.websocketLimitsValue', 'Max {max} conn/IP | {idle}min idle timeout | {size}KB max msg', {
            max: val.max_per_ip || 5,
            idle: Math.round((val.idle_timeout_secs || 1800) / 60),
            size: Math.round((val.max_message_size || 65536) / 1024)
          });
        case 'wasm_sandbox':
          return this.t('settingsPage.wasmSandboxValue', 'Fuel: {fuel} | Epoch: {epoch} | Timeout: {timeout}s', {
            fuel: val.fuel_metering ? this.t('settingsPage.on', 'ON') : this.t('settingsPage.off', 'OFF'),
            epoch: val.epoch_interruption ? this.t('settingsPage.on', 'ON') : this.t('settingsPage.off', 'OFF'),
            timeout: val.default_timeout_secs || 30
          });
        case 'auth':
          return this.t('settingsPage.authValue', 'Mode: {mode}{suffix}', {
            mode: val.mode || this.t('status.unknown', 'unknown'),
            suffix: val.api_key_set
              ? this.t('settingsPage.keyConfiguredSuffix', ' (key configured)')
              : this.t('settingsPage.noKeySetSuffix', ' (no key set)')
          });
        default:
          return this.configurableFeatureHint(feature);
      }
    },

    formatMonitoringValue(feature) {
      var val = this.getMonitoringValue(feature.valueKey);
      if (!val) return this.monitoringFeatureHint(feature);
      switch (feature.valueKey) {
        case 'audit_trail':
          return this.t('settingsPage.auditTrailValue', '{state} | {algorithm} | {count} entries logged', {
            state: this.securityStateText(val.enabled),
            algorithm: val.algorithm || 'SHA-256',
            count: val.entry_count || 0
          });
        case 'taint_tracking':
          var labels = val.tracked_labels || [];
          return this.t('settingsPage.taintTrackingValue', '{state} | Tracking: {labels}', {
            state: this.securityStateText(val.enabled),
            labels: labels.join(', ')
          });
        case 'manifest_signing':
          return this.t('settingsPage.manifestSigningValue', 'Algorithm: {algorithm} | {availability}', {
            algorithm: val.algorithm || 'Ed25519',
            availability: this.monitoringAvailableText(val.available)
          });
        default:
          return this.monitoringFeatureHint(feature);
      }
    },

    async verifyAuditChain() {
      this.verifyingChain = true;
      this.chainResult = null;
      try {
        var res = await LibreFangAPI.get('/api/audit/verify');
        this.chainResult = res;
      } catch(e) {
        this.chainResult = { valid: false, error: e.message };
      }
      this.verifyingChain = false;
    },

    // -- Peers methods --
    async loadPeers() {
      this.peersLoading = true;
      this.peersLoadError = '';
      try {
        var data = await LibreFangAPI.get('/api/peers');
        this.peers = (data.peers || []).map(function(p) {
          return {
            node_id: p.node_id,
            node_name: p.node_name,
            address: p.address,
            state: p.state,
            agent_count: (p.agents || []).length,
            protocol_version: p.protocol_version || 1
          };
        });
      } catch(e) {
        this.peers = [];
        this.peersLoadError = e.message || this.t('settingsPage.loadPeersFailed', 'Could not load peers.');
      }
      this.peersLoading = false;
    },

    startPeerPolling() {
      var self = this;
      this.stopPeerPolling();
      this._peerPollTimer = setInterval(async function() {
        if (self.tab !== 'network') { self.stopPeerPolling(); return; }
        try {
          var data = await LibreFangAPI.get('/api/peers');
          self.peers = (data.peers || []).map(function(p) {
            return {
              node_id: p.node_id,
              node_name: p.node_name,
              address: p.address,
              state: p.state,
              agent_count: (p.agents || []).length,
              protocol_version: p.protocol_version || 1
            };
          });
        } catch(e) { /* silent */ }
      }, 15000);
    },

    stopPeerPolling() {
      if (this._peerPollTimer) { clearInterval(this._peerPollTimer); this._peerPollTimer = null; }
    },

    // -- Migration methods --
    async autoDetect() {
      this.detecting = true;
      try {
        var data = await LibreFangAPI.get('/api/migrate/detect');
        if (data.detected && data.scan) {
          this.sourcePath = data.path;
          this.scanResult = data.scan;
          this.migStep = 'preview';
        } else {
          this.migStep = 'not_found';
        }
      } catch(e) {
        this.migStep = 'not_found';
      }
      this.detecting = false;
    },

    async scanPath() {
      if (!this.sourcePath) return;
      this.scanning = true;
      try {
        var data = await LibreFangAPI.post('/api/migrate/scan', { path: this.sourcePath });
        if (data.error) {
          LibreFangToast.error(this.t('settingsPage.scanError', 'Scan error: {message}', { message: data.error }));
          this.scanning = false;
          return;
        }
        this.scanResult = data;
        this.migStep = 'preview';
      } catch(e) {
        LibreFangToast.error(this.t('settingsPage.scanFailed', 'Scan failed: {message}', { message: e.message }));
      }
      this.scanning = false;
    },

    async runMigration(dryRun) {
      this.migrating = true;
      try {
        var target = this.targetPath;
        if (!target) target = '';
        var data = await LibreFangAPI.post('/api/migrate', {
          source: 'openclaw',
          source_dir: this.sourcePath || (this.scanResult ? this.scanResult.path : ''),
          target_dir: target,
          dry_run: dryRun
        });
        this.migResult = data;
        this.migStep = 'result';
      } catch(e) {
        this.migResult = { status: 'failed', error: e.message };
        this.migStep = 'result';
      }
      this.migrating = false;
    },

    // -- Model Catalog Sync --
    async fetchCatalogStatus() {
      try {
        var data = await LibreFangAPI.get('/api/catalog/status');
        this.catalogStatus = data;
      } catch(e) {
        console.error('Failed to fetch catalog status:', e);
      }
    },

    async updateCatalog() {
      this.catalogUpdating = true;
      this.catalogResult = '';
      try {
        var data = await LibreFangAPI.post('/api/catalog/update', {});
        if (data.status === 'ok') {
          this.catalogResult = this.t('settingsPage.catalogUpdated', 'Updated: {files} files, {models} models', {
            files: data.files_downloaded,
            models: data.models_count
          });
          this.catalogStatus.last_sync = data.timestamp;
          // Refresh models list to pick up new catalog entries
          this.loadModels();
        } else {
          this.catalogResult = this.t('settingsPage.catalogError', 'Error: {message}', {
            message: data.message || 'Unknown error'
          });
        }
      } catch(e) {
        this.catalogResult = this.t('settingsPage.catalogError', 'Error: {message}', {
          message: e.message || 'Request failed'
        });
      }
      this.catalogUpdating = false;
    },

    catalogLastSyncText() {
      if (this.catalogStatus.last_sync) {
        return this.t('settingsPage.lastSynced', 'Last synced: {time}', {
          time: new Date(this.catalogStatus.last_sync).toLocaleString()
        });
      }
      return this.t('settingsPage.neverSynced', 'Never synced');
    },

    destroy() {
      this.stopPeerPolling();
    }
  };
}
