// LibreFang Agents Page — Multi-step spawn wizard, detail view with tabs, file editor, personality presets
'use strict';

/** Escape a string for use inside TOML triple-quoted strings ("""\n...\n""").
 *  Backslashes are escaped, and runs of 3+ consecutive double-quotes are
 *  broken up so the TOML parser never sees an unintended closing delimiter.
 */
function tomlMultilineEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"""/g, '""\\"');
}

/** Escape a string for use inside a TOML basic (single-line) string ("...").
 *  Backslashes, double-quotes, and common control chars are escaped.
 */
function tomlBasicEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
}

function agentsPage() {
  return {
    tab: 'agents',
    activeChatAgent: null,
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
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
    // -- Agents state --
    showSpawnModal: false,
    showDetailModal: false,
    detailAgent: null,
    spawnMode: 'wizard',
    spawning: false,
    spawnToml: '',
    filterState: 'all',
    loading: true,
    loadError: '',
    // Default model from config (fetched in init, fallback to groq/llama)
    _defaultProvider: 'groq',
    _defaultModel: 'llama-3.3-70b-versatile',
    spawnForm: {
      name: '',
      provider: 'groq',
      model: 'llama-3.3-70b-versatile',
      systemPrompt: 'You are a helpful assistant.',
      profile: 'full',
      caps: { memory_read: true, memory_write: true, network: false, shell: false, agent_spawn: false }
    },

    // -- Multi-step wizard state --
    spawnStep: 1,
    spawnIdentity: { emoji: '', color: '#FF5C00', archetype: '' },
    selectedPreset: '',
    soulContent: '',
    emojiOptions: [
      '\u{1F916}', '\u{1F4BB}', '\u{1F50D}', '\u{270D}\uFE0F', '\u{1F4CA}', '\u{1F6E0}\uFE0F',
      '\u{1F4AC}', '\u{1F393}', '\u{1F310}', '\u{1F512}', '\u{26A1}', '\u{1F680}',
      '\u{1F9EA}', '\u{1F3AF}', '\u{1F4D6}', '\u{1F9D1}\u200D\u{1F4BB}', '\u{1F4E7}', '\u{1F3E2}',
      '\u{2764}\uFE0F', '\u{1F31F}', '\u{1F527}', '\u{1F4DD}', '\u{1F4A1}', '\u{1F3A8}'
    ],
    archetypeOptions: ['Assistant', 'Researcher', 'Coder', 'Writer', 'DevOps', 'Support', 'Analyst', 'Custom'],
    personalityPresets: [
      { id: 'professional', label: 'Professional', soul: 'Communicate in a clear, professional tone. Be direct and structured. Use formal language and data-driven reasoning. Prioritize accuracy over personality.' },
      { id: 'friendly', label: 'Friendly', soul: 'Be warm, approachable, and conversational. Use casual language and show genuine interest in the user. Add personality to your responses while staying helpful.' },
      { id: 'technical', label: 'Technical', soul: 'Focus on technical accuracy and depth. Use precise terminology. Show your work and reasoning. Prefer code examples and structured explanations.' },
      { id: 'creative', label: 'Creative', soul: 'Be imaginative and expressive. Use vivid language, analogies, and unexpected connections. Encourage creative thinking and explore multiple perspectives.' },
      { id: 'concise', label: 'Concise', soul: 'Be extremely brief and to the point. No filler, no pleasantries. Answer in the fewest words possible while remaining accurate and complete.' },
      { id: 'mentor', label: 'Mentor', soul: 'Be patient and encouraging like a great teacher. Break down complex topics step by step. Ask guiding questions. Celebrate progress and build confidence.' }
    ],

    // -- Detail modal tabs --
    detailTab: 'info',
    agentFiles: [],
    editingFile: null,
    fileContent: '',
    fileSaving: false,
    filesLoading: false,
    configForm: {},
    configSaving: false,
    // -- Tool filters --
    toolFilters: { tool_allowlist: [], tool_blocklist: [] },
    toolFiltersLoading: false,
    newAllowTool: '',
    newBlockTool: '',
    // -- Model switch --
    editingModel: false,
    newModelValue: '',
    modelSaving: false,
    // -- Fallback chain --
    editingFallback: false,
    newFallbackValue: '',

    // -- Templates state --
    tplTemplates: [],
    tplProviders: [],
    tplLoading: false,
    tplLoadError: '',
    selectedCategory: 'All',
    searchQuery: '',

    builtinTemplates: [
      {
        name: 'General Assistant',
        description: 'A versatile conversational agent that can help with everyday tasks, answer questions, and provide recommendations.',
        category: 'General',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a helpful, friendly assistant. Provide clear, accurate, and concise responses. Ask clarifying questions when needed.'
      },
      {
        name: 'Code Helper',
        description: 'A programming-focused agent that writes, reviews, and debugs code across multiple languages.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.'
      },
      {
        name: 'Researcher',
        description: 'An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries.',
        category: 'Research',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'research',
        system_prompt: 'You are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.'
      },
      {
        name: 'Writer',
        description: 'A creative writing agent that helps with drafting, editing, and improving written content of all kinds.',
        category: 'Writing',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.'
      },
      {
        name: 'Data Analyst',
        description: 'A data-focused agent that helps analyze datasets, create queries, and interpret statistical results.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.'
      },
      {
        name: 'DevOps Engineer',
        description: 'A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'automation',
        system_prompt: 'You are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.'
      },
      {
        name: 'Customer Support',
        description: 'A professional, empathetic agent for handling customer inquiries and resolving issues.',
        category: 'Business',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'messaging',
        system_prompt: 'You are a professional customer support representative. Be empathetic, patient, and solution-oriented. Acknowledge concerns before offering solutions. Escalate complex issues appropriately.'
      },
      {
        name: 'Tutor',
        description: 'A patient educational agent that explains concepts step-by-step and adapts to the learner\'s level.',
        category: 'General',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'full',
        system_prompt: 'You are a patient and encouraging tutor. Explain concepts step by step, starting from fundamentals. Use analogies and examples. Check understanding before moving on. Adapt to the learner\'s pace.'
      },
      {
        name: 'API Designer',
        description: 'An agent specialized in RESTful API design, OpenAPI specs, and integration architecture.',
        category: 'Development',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'coding',
        system_prompt: 'You are an API design expert. Help users design clean, consistent RESTful APIs following best practices. Cover endpoint naming, request/response schemas, error handling, and versioning.'
      },
      {
        name: 'Meeting Notes',
        description: 'Summarizes meeting transcripts into structured notes with action items and key decisions.',
        category: 'Business',
        provider: 'groq',
        model: 'llama-3.3-70b-versatile',
        profile: 'minimal',
        system_prompt: 'You are a meeting summarizer. When given a meeting transcript or notes, produce a structured summary with: key decisions, action items (with owners), discussion highlights, and follow-up questions.'
      }
    ],

    get localizedTemplates() {
      var lang = this._currentLang;
      if (typeof i18n === 'undefined' || lang === 'en') {
        return this.builtinTemplates;
      }

      return this.builtinTemplates.map(function(template) {
        var key = template.name.replace(/\s+/g, '');
        var translatedName = i18n.t('template.' + key + '.name');
        var translatedDesc = i18n.t('template.' + key + '.desc');
        var translatedCategory = i18n.t('category.' + template.category.toLowerCase());
        return {
          name: translatedName && !translatedName.startsWith('[') ? translatedName : template.name,
          description: translatedDesc && !translatedDesc.startsWith('[') ? translatedDesc : template.description,
          category: translatedCategory && !translatedCategory.startsWith('[') ? translatedCategory : template.category,
          provider: template.provider,
          model: template.model,
          profile: template.profile,
          system_prompt: template.system_prompt
        };
      });
    },

    // ── Profile Descriptions ──
    profileDescriptions: {
      minimal: { label: 'Minimal', desc: 'Read-only file access' },
      coding: { label: 'Coding', desc: 'Files + shell + web fetch' },
      research: { label: 'Research', desc: 'Web search + file read/write' },
      messaging: { label: 'Messaging', desc: 'Agents + memory access' },
      automation: { label: 'Automation', desc: 'All tools except custom' },
      balanced: { label: 'Balanced', desc: 'General-purpose tool set' },
      precise: { label: 'Precise', desc: 'Focused tool set for accuracy' },
      creative: { label: 'Creative', desc: 'Full tools with creative emphasis' },
      full: { label: 'Full', desc: 'All 35+ tools' }
    },
    profileInfo: function(name) {
      var info = this.profileDescriptions[name] || { label: name, desc: '' };
      return {
        label: this.t('profile.' + name + '.label', info.label),
        desc: this.t('profile.' + name + '.desc', info.desc)
      };
    },
    archetypeLabel(name) {
      if (!name) return this.t('detail.noneValue', 'None');
      return this.t('agentsPage.archetype.' + name.toLowerCase(), name);
    },
    vibeLabel(name) {
      if (!name) return this.t('detail.noneValue', 'None');
      return this.t('agentsPage.vibe.' + name.toLowerCase(), name);
    },
    modeLabel(name) {
      if (!name) return this.t('detail.noneValue', 'None');
      return this.t('mode.' + name.toLowerCase(), name);
    },
    personalityLabel(preset) {
      return this.t('agentsPage.vibe.' + preset.id, preset.label);
    },
    fileStatusText(file) {
      if (!file || !file.exists) return this.t('detail.notCreated', 'Not created');
      return this.t('agentsPage.fileBytes', '{count} bytes', { count: file.size_bytes || 0 });
    },
    removeToolTitle(tool) {
      return this.t('detail.removeTool', 'Click to remove {tool}', { tool: tool });
    },

    // ── Tool Preview in Spawn Modal ──
    spawnProfiles: [],
    spawnProfilesLoaded: false,
    async loadSpawnProfiles() {
      if (this.spawnProfilesLoaded) return;
      try {
        var data = await LibreFangAPI.get('/api/profiles');
        this.spawnProfiles = data.profiles || [];
        this.spawnProfilesLoaded = true;
      } catch(e) { this.spawnProfiles = []; }
    },
    get selectedProfileTools() {
      var pname = this.spawnForm.profile;
      var match = this.spawnProfiles.find(function(p) { return p.name === pname; });
      if (match && match.tools) return match.tools.slice(0, 15);
      return [];
    },

    get agents() { return Alpine.store('app').agents; },

    get filteredAgents() {
      var f = this.filterState;
      if (f === 'all') return this.agents;
      return this.agents.filter(function(a) { return a.state.toLowerCase() === f; });
    },

    get runningCount() {
      return this.agents.filter(function(a) { return a.state === 'Running'; }).length;
    },

    get stoppedCount() {
      return this.agents.filter(function(a) { return a.state !== 'Running'; }).length;
    },

    // -- Templates computed --
    get categories() {
      var cats = { 'All': true };
      this.builtinTemplates.forEach(function(t) { cats[t.category] = true; });
      this.tplTemplates.forEach(function(t) { if (t.category) cats[t.category] = true; });
      return Object.keys(cats);
    },

    get filteredBuiltins() {
      var self = this;
      return this.builtinTemplates.filter(function(t) {
        if (self.selectedCategory !== 'All' && t.category !== self.selectedCategory) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if (t.name.toLowerCase().indexOf(q) === -1 &&
              t.description.toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    get filteredCustom() {
      var self = this;
      return this.tplTemplates.filter(function(t) {
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if ((t.name || '').toLowerCase().indexOf(q) === -1 &&
              (t.description || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    isProviderConfigured(providerName) {
      if (!providerName) return false;
      var p = this.tplProviders.find(function(pr) { return pr.id === providerName; });
      return p ? p.auth_status === 'configured' : false;
    },

    _updateURL() {
      var params = [];
      if (this.tab && this.tab !== 'agents') params.push('tab=' + encodeURIComponent(this.tab));
      if (this.filterState && this.filterState !== 'all') params.push('filter=' + encodeURIComponent(this.filterState));
      var hash = 'agents' + (params.length ? '?' + params.join('&') : '');
      if (window.location.hash !== '#' + hash) history.replaceState(null, '', '#' + hash);
    },

    async init() {
      var self = this;
      // Fetch default model from config so we don't hardcode it
      try {
        var config = await LibreFangAPI.get('/api/config');
        if (config && config.default_model) {
          if (config.default_model.provider) self._defaultProvider = config.default_model.provider;
          if (config.default_model.model) self._defaultModel = config.default_model.model;
          // Update spawnForm defaults
          self.spawnForm.provider = self._defaultProvider;
          self.spawnForm.model = self._defaultModel;
          // Update builtin templates to use configured default
          self.builtinTemplates.forEach(function(t) {
            t.provider = self._defaultProvider;
            t.model = self._defaultModel;
          });
        }
      } catch(e) { /* use hardcoded fallbacks */ }
      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
      });
      var hashParts = window.location.hash.split('?');
      if (hashParts.length > 1) {
        var params = new URLSearchParams(hashParts[1]);
        if (params.get('tab')) self.tab = params.get('tab');
        if (params.get('filter')) self.filterState = params.get('filter');
      }
      this.$watch('tab', function() { self._updateURL(); });
      this.$watch('filterState', function() { self._updateURL(); });
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        this.loadError = e.message || this.t('agentsPage.loadAgentsFailed', 'Could not load agents. Is the daemon running?');
      }
      this.loading = false;

      // If a pending agent was set (e.g. from wizard or redirect), open chat inline
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        this.activeChatAgent = store.pendingAgent;
      }
      // Watch for future pendingAgent changes
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.activeChatAgent = agent;
        }
      });
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        this.loadError = e.message || this.t('agentsPage.loadAgentsFailedShort', 'Could not load agents.');
      }
      this.loading = false;
    },

    async loadTemplates() {
      this.tplLoading = true;
      this.tplLoadError = '';
      try {
        var results = await Promise.all([
          LibreFangAPI.get('/api/templates'),
          LibreFangAPI.get('/api/providers').catch(function() { return { providers: [] }; })
        ]);
        this.tplTemplates = results[0].templates || [];
        this.tplProviders = results[1].providers || [];
      } catch(e) {
        this.tplTemplates = [];
        this.tplLoadError = e.message || this.t('agentsPage.loadTemplatesFailed', 'Could not load templates.');
      }
      this.tplLoading = false;
    },

    chatWithAgent(agent) {
      Alpine.store('app').pendingAgent = agent;
      this.activeChatAgent = agent;
    },

    closeChat() {
      this.activeChatAgent = null;
      LibreFangAPI.wsDisconnect();
    },

    async showDetail(agent) {
      this.detailAgent = agent;
      this.detailAgent._fallbacks = [];
      this.detailTab = 'info';
      this.agentFiles = [];
      this.editingFile = null;
      this.fileContent = '';
      this.editingFallback = false;
      this.newFallbackValue = '';
      this.configForm = {
        name: agent.name || '',
        system_prompt: agent.system_prompt || '',
        emoji: (agent.identity && agent.identity.emoji) || '',
        color: (agent.identity && agent.identity.color) || '#FF5C00',
        archetype: (agent.identity && agent.identity.archetype) || '',
        vibe: (agent.identity && agent.identity.vibe) || ''
      };
      this.showDetailModal = true;
      // Fetch full agent detail to get fallback_models
      try {
        var full = await LibreFangAPI.get('/api/agents/' + agent.id);
        this.detailAgent._fallbacks = full.fallback_models || [];
      } catch(e) { /* ignore */ }
    },

    killAgent(agent) {
      var self = this;
      LibreFangToast.confirm(
        self.t('agentsPage.stopAgentTitle', 'Stop Agent'),
        self.t('agentsPage.stopAgentConfirm', 'Stop agent "{name}"? The agent will be shut down.', { name: agent.name }),
        async function() {
        try {
          await LibreFangAPI.del('/api/agents/' + agent.id);
          LibreFangToast.success(self.t('agentsPage.agentStopped', 'Agent "{name}" stopped', { name: agent.name }));
          self.showDetailModal = false;
          await Alpine.store('app').refreshAgents();
        } catch(e) {
          LibreFangToast.error(self.t('agentsPage.stopAgentFailed', 'Failed to stop agent: {message}', { message: e.message }));
        }
      });
    },

    killAllAgents() {
      var list = this.filteredAgents;
      if (!list.length) return;
      var self = this;
      LibreFangToast.confirm(
        this.t('agentsPage.stopAllTitle', 'Stop All Agents'),
        this.t('agentsPage.stopAllConfirm', 'Stop {count} agent(s)? All agents will be shut down.', { count: list.length }),
        async function() {
        var errors = [];
        for (var i = 0; i < list.length; i++) {
          try {
            await LibreFangAPI.del('/api/agents/' + list[i].id);
          } catch(e) { errors.push(list[i].name + ': ' + e.message); }
        }
        await Alpine.store('app').refreshAgents();
        if (errors.length) {
          LibreFangToast.error(self.t('agentsPage.stopSomeFailed', 'Some agents failed to stop: {errors}', { errors: errors.join(', ') }));
        } else {
          LibreFangToast.success(self.t('agentsPage.agentsStopped', '{count} agent(s) stopped', { count: list.length }));
        }
      });
    },

    // ── Multi-step wizard navigation ──
    openSpawnWizard() {
      this.showSpawnModal = true;
      this.spawnStep = 1;
      this.spawnMode = 'wizard';
      this.spawnIdentity = { emoji: '', color: '#FF5C00', archetype: '' };
      this.selectedPreset = '';
      this.soulContent = '';
      this.spawnForm.name = '';
      this.spawnForm.provider = this._defaultProvider;
      this.spawnForm.model = this._defaultModel;
      this.spawnForm.systemPrompt = 'You are a helpful assistant.';
      this.spawnForm.profile = 'full';
    },

    nextStep() {
      if (this.spawnStep === 1 && !this.spawnForm.name.trim()) {
        LibreFangToast.warn(this.t('agentsPage.pleaseEnterAgentName', 'Please enter an agent name'));
        return;
      }
      if (this.spawnStep < 5) this.spawnStep++;
    },

    prevStep() {
      if (this.spawnStep > 1) this.spawnStep--;
    },

    selectPreset(preset) {
      this.selectedPreset = preset.id;
      this.soulContent = preset.soul;
    },

    generateToml() {
      var f = this.spawnForm;
      var si = this.spawnIdentity;
      var lines = [
        'name = "' + tomlBasicEscape(f.name) + '"',
        'module = "builtin:chat"'
      ];
      if (f.profile && f.profile !== 'custom') {
        lines.push('profile = "' + f.profile + '"');
      }
      lines.push('', '[model]');
      lines.push('provider = "' + f.provider + '"');
      lines.push('model = "' + f.model + '"');
      lines.push('system_prompt = """\n' + tomlMultilineEscape(f.systemPrompt) + '\n"""');
      if (f.profile === 'custom') {
        lines.push('', '[capabilities]');
        if (f.caps.memory_read) lines.push('memory_read = ["*"]');
        if (f.caps.memory_write) lines.push('memory_write = ["self.*"]');
        if (f.caps.network) lines.push('network = ["*"]');
        if (f.caps.shell) lines.push('shell = ["*"]');
        if (f.caps.agent_spawn) lines.push('agent_spawn = true');
      }
      return lines.join('\n');
    },

    async setMode(agent, mode) {
      try {
        await LibreFangAPI.put('/api/agents/' + agent.id + '/mode', { mode: mode });
        agent.mode = mode;
        LibreFangToast.success(this.t('agentsPage.modeSet', 'Mode set to {mode}', { mode: this.modeLabel(mode) }));
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.setModeFailed', 'Failed to set mode: {message}', { message: e.message }));
      }
    },

    async spawnAgent() {
      this.spawning = true;
      var toml = this.spawnMode === 'wizard' ? this.generateToml() : this.spawnToml;
      if (!toml.trim()) {
        this.spawning = false;
        LibreFangToast.warn(this.t('agentsPage.manifestEmpty', 'Manifest is empty - enter agent config first'));
        return;
      }

      try {
        var res = await LibreFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          // Post-spawn: update identity + write SOUL.md if personality preset selected
          var patchBody = {};
          if (this.spawnIdentity.emoji) patchBody.emoji = this.spawnIdentity.emoji;
          if (this.spawnIdentity.color) patchBody.color = this.spawnIdentity.color;
          if (this.spawnIdentity.archetype) patchBody.archetype = this.spawnIdentity.archetype;
          if (this.selectedPreset) patchBody.vibe = this.selectedPreset;

          if (Object.keys(patchBody).length) {
            LibreFangAPI.patch('/api/agents/' + res.agent_id + '/config', patchBody).catch(function(e) { console.warn('Post-spawn config patch failed:', e.message); });
          }
          if (this.soulContent.trim()) {
            LibreFangAPI.put('/api/agents/' + res.agent_id + '/files/SOUL.md', { content: '# Soul\n' + this.soulContent }).catch(function(e) { console.warn('SOUL.md write failed:', e.message); });
          }

          this.showSpawnModal = false;
          this.spawnForm.name = '';
          this.spawnToml = '';
          this.spawnStep = 1;
          LibreFangToast.success(this.t('agentsPage.agentSpawned', 'Agent "{name}" spawned', { name: res.name || this.t('agentsPage.newAgentName', 'new') }));
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: res.name, model_provider: '?', model_name: '?' });
        } else {
          LibreFangToast.error(this.t('agentsPage.spawnFailed', 'Spawn failed: {message}', { message: res.error || this.t('agentsPage.unknownError', 'Unknown error') }));
        }
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.spawnAgentFailed', 'Failed to spawn agent: {message}', { message: e.message }));
      }
      this.spawning = false;
    },

    // ── Detail modal: Files tab ──
    async loadAgentFiles() {
      if (!this.detailAgent) return;
      this.filesLoading = true;
      try {
        var data = await LibreFangAPI.get('/api/agents/' + this.detailAgent.id + '/files');
        this.agentFiles = data.files || [];
      } catch(e) {
        this.agentFiles = [];
        LibreFangToast.error(this.t('agentsPage.loadFilesFailed', 'Failed to load files: {message}', { message: e.message }));
      }
      this.filesLoading = false;
    },

    async openFile(file) {
      if (!file.exists) {
        // Create with empty content
        this.editingFile = file.name;
        this.fileContent = '';
        return;
      }
      try {
        var data = await LibreFangAPI.get('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(file.name));
        this.editingFile = file.name;
        this.fileContent = data.content || '';
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.readFileFailed', 'Failed to read file: {message}', { message: e.message }));
      }
    },

    async saveFile() {
      if (!this.editingFile || !this.detailAgent) return;
      this.fileSaving = true;
      try {
        await LibreFangAPI.put('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(this.editingFile), { content: this.fileContent });
        LibreFangToast.success(this.t('agentsPage.fileSaved', '{file} saved', { file: this.editingFile }));
        await this.loadAgentFiles();
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.saveFileFailed', 'Failed to save file: {message}', { message: e.message }));
      }
      this.fileSaving = false;
    },

    closeFileEditor() {
      this.editingFile = null;
      this.fileContent = '';
    },

    // ── Detail modal: Config tab ──
    async saveConfig() {
      if (!this.detailAgent) return;
      this.configSaving = true;
      try {
        await LibreFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', this.configForm);
        LibreFangToast.success(this.t('agentsPage.configUpdated', 'Config updated'));
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.saveConfigFailed', 'Failed to save config: {message}', { message: e.message }));
      }
      this.configSaving = false;
    },

    // ── Clone agent ──
    async cloneAgent(agent) {
      var newName = (agent.name || 'agent') + '-copy';
      try {
        var res = await LibreFangAPI.post('/api/agents/' + agent.id + '/clone', { new_name: newName });
        if (res.agent_id) {
          LibreFangToast.success(this.t('agentsPage.clonedAs', 'Cloned as "{name}"', { name: res.name }));
          await Alpine.store('app').refreshAgents();
          this.showDetailModal = false;
        }
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.cloneFailed', 'Clone failed: {message}', { message: e.message }));
      }
    },

    // -- Template methods --
    async spawnFromTemplate(name) {
      try {
        var data = await LibreFangAPI.get('/api/templates/' + encodeURIComponent(name));
        if (data.manifest_toml) {
          var res = await LibreFangAPI.post('/api/agents', { manifest_toml: data.manifest_toml });
          if (res.agent_id) {
            LibreFangToast.success(this.t('agentsPage.spawnedFromTemplate', 'Agent "{name}" spawned from template', { name: res.name || name }));
            await Alpine.store('app').refreshAgents();
            this.chatWithAgent({ id: res.agent_id, name: res.name || name, model_provider: '?', model_name: '?' });
          }
        }
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.spawnFromTemplateFailed', 'Failed to spawn from template: {message}', { message: e.message }));
      }
    },

    // ── Clear agent history ──
    async clearHistory(agent) {
      var self = this;
      LibreFangToast.confirm(
        self.t('agentsPage.clearHistoryTitle', 'Clear History'),
        self.t('agentsPage.clearHistoryConfirm', 'Clear all conversation history for "{name}"? This cannot be undone.', { name: agent.name }),
        async function() {
        try {
          await LibreFangAPI.del('/api/agents/' + agent.id + '/history');
          LibreFangToast.success(self.t('agentsPage.historyCleared', 'History cleared for "{name}"', { name: agent.name }));
        } catch(e) {
          LibreFangToast.error(self.t('agentsPage.clearHistoryFailed', 'Failed to clear history: {message}', { message: e.message }));
        }
      });
    },

    // ── Model switch ──
    async changeModel() {
      if (!this.detailAgent || !this.newModelValue.trim()) return;
      this.modelSaving = true;
      try {
        var resp = await LibreFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: this.newModelValue.trim() });
        var providerInfo = (resp && resp.provider)
          ? this.t('agentsPage.providerSuffix', ' (provider: {provider})', { provider: resp.provider })
          : '';
        LibreFangToast.success(this.t('agentsPage.modelChanged', 'Model changed{provider} (memory reset)', { provider: providerInfo }));
        this.editingModel = false;
        await Alpine.store('app').refreshAgents();
        // Refresh detailAgent
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.changeModelFailed', 'Failed to change model: {message}', { message: e.message }));
      }
      this.modelSaving = false;
    },

    // ── Fallback model chain ──
    async addFallback() {
      if (!this.detailAgent || !this.newFallbackValue.trim()) return;
      var parts = this.newFallbackValue.trim().split('/');
      var provider = parts.length > 1 ? parts[0] : this.detailAgent.model_provider;
      var model = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
      if (!this.detailAgent._fallbacks) this.detailAgent._fallbacks = [];
      this.detailAgent._fallbacks.push({ provider: provider, model: model });
      try {
        await LibreFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        LibreFangToast.success(this.t('agentsPage.fallbackAdded', 'Fallback added: {value}', { value: provider + '/' + model }));
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.saveFallbacksFailed', 'Failed to save fallbacks: {message}', { message: e.message }));
        this.detailAgent._fallbacks.pop();
      }
      this.editingFallback = false;
      this.newFallbackValue = '';
    },

    async removeFallback(idx) {
      if (!this.detailAgent || !this.detailAgent._fallbacks) return;
      var removed = this.detailAgent._fallbacks.splice(idx, 1);
      try {
        await LibreFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        LibreFangToast.success(this.t('agentsPage.fallbackRemoved', 'Fallback removed'));
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.saveFallbacksFailed', 'Failed to save fallbacks: {message}', { message: e.message }));
        this.detailAgent._fallbacks.splice(idx, 0, removed[0]);
      }
    },

    // ── Tool filters ──
    async loadToolFilters() {
      if (!this.detailAgent) return;
      this.toolFiltersLoading = true;
      try {
        this.toolFilters = await LibreFangAPI.get('/api/agents/' + this.detailAgent.id + '/tools');
      } catch(e) {
        this.toolFilters = { tool_allowlist: [], tool_blocklist: [] };
      }
      this.toolFiltersLoading = false;
    },

    addAllowTool() {
      var t = this.newAllowTool.trim();
      if (t && this.toolFilters.tool_allowlist.indexOf(t) === -1) {
        this.toolFilters.tool_allowlist.push(t);
        this.newAllowTool = '';
        this.saveToolFilters();
      }
    },

    removeAllowTool(tool) {
      this.toolFilters.tool_allowlist = this.toolFilters.tool_allowlist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    addBlockTool() {
      var t = this.newBlockTool.trim();
      if (t && this.toolFilters.tool_blocklist.indexOf(t) === -1) {
        this.toolFilters.tool_blocklist.push(t);
        this.newBlockTool = '';
        this.saveToolFilters();
      }
    },

    removeBlockTool(tool) {
      this.toolFilters.tool_blocklist = this.toolFilters.tool_blocklist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    async saveToolFilters() {
      if (!this.detailAgent) return;
      try {
        await LibreFangAPI.put('/api/agents/' + this.detailAgent.id + '/tools', this.toolFilters);
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.updateToolFiltersFailed', 'Failed to update tool filters: {message}', { message: e.message }));
      }
    },

    async spawnBuiltin(t) {
      var toml = 'name = "' + tomlBasicEscape(t.name) + '"\n';
      toml += 'description = "' + tomlBasicEscape(t.description) + '"\n';
      toml += 'module = "builtin:chat"\n';
      toml += 'profile = "' + t.profile + '"\n\n';
      toml += '[model]\nprovider = "' + t.provider + '"\nmodel = "' + t.model + '"\n';
      toml += 'system_prompt = """\n' + tomlMultilineEscape(t.system_prompt) + '\n"""\n';

      try {
        var res = await LibreFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          LibreFangToast.success(this.t('agentsPage.agentSpawned', 'Agent "{name}" spawned', { name: t.name }));
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: t.name, model_provider: t.provider, model_name: t.model });
        }
      } catch(e) {
        LibreFangToast.error(this.t('agentsPage.spawnBuiltinFailed', 'Failed to spawn agent: {message}', { message: e.message }));
      }
    }
  };
}
