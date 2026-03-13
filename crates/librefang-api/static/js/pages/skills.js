// LibreFang Skills Page — OpenClaw/ClawHub ecosystem + local skills + MCP servers
'use strict';

function skillsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    tab: 'installed',
    skills: [],
    loading: true,
    loadError: '',

    // ClawHub state
    clawhubSearch: '',
    clawhubResults: [],
    clawhubBrowseResults: [],
    clawhubLoading: false,
    clawhubError: '',
    clawhubSort: 'trending',
    clawhubNextCursor: null,
    installingSlug: null,
    installResult: null,
    _searchTimer: null,
    _browseCache: {},    // { key: { ts, data } } client-side 60s cache
    _searchCache: {},

    // Skill detail modal
    skillDetail: null,
    detailLoading: false,
    showSkillCode: false,
    skillCode: '',
    skillCodeFilename: '',
    skillCodeLoading: false,

    // MCP servers
    mcpServers: [],
    mcpLoading: false,

    // Category definitions from the OpenClaw ecosystem
    categories: [
      { id: 'coding', name: 'Coding & IDEs' },
      { id: 'git', name: 'Git & GitHub' },
      { id: 'web', name: 'Web & Frontend' },
      { id: 'devops', name: 'DevOps & Cloud' },
      { id: 'browser', name: 'Browser & Automation' },
      { id: 'search', name: 'Search & Research' },
      { id: 'ai', name: 'AI & LLMs' },
      { id: 'data', name: 'Data & Analytics' },
      { id: 'productivity', name: 'Productivity' },
      { id: 'communication', name: 'Communication' },
      { id: 'media', name: 'Media & Streaming' },
      { id: 'notes', name: 'Notes & PKM' },
      { id: 'security', name: 'Security' },
      { id: 'cli', name: 'CLI Utilities' },
      { id: 'marketing', name: 'Marketing & Sales' },
      { id: 'finance', name: 'Finance' },
      { id: 'smart-home', name: 'Smart Home & IoT' },
      { id: 'docs', name: 'PDF & Documents' },
    ],

    init() {
      var self = this;
      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
      });
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

    categoryLabel(cat) {
      return this.t('skillsPage.category.' + cat.id, cat.name);
    },

    runtimeBadge: function(rt) {
      var r = (rt || '').toLowerCase();
      if (r === 'python' || r === 'py') return { text: 'PY', cls: 'runtime-badge-py' };
      if (r === 'node' || r === 'nodejs' || r === 'js' || r === 'javascript') return { text: 'JS', cls: 'runtime-badge-js' };
      if (r === 'wasm' || r === 'webassembly') return { text: 'WASM', cls: 'runtime-badge-wasm' };
      if (r === 'prompt_only' || r === 'prompt' || r === 'promptonly') {
        return { text: this.t('skillsPage.runtime.prompt', 'PROMPT'), cls: 'runtime-badge-prompt' };
      }
      return { text: r.toUpperCase().substring(0, 4), cls: 'runtime-badge-prompt' };
    },

    sourceBadge: function(source) {
      if (!source) return { text: this.t('skillsPage.source.local', 'Local'), cls: 'badge-dim' };
      switch (source.type) {
        case 'clawhub': return { text: this.t('skillsPage.source.clawhub', 'ClawHub'), cls: 'badge-info' };
        case 'openclaw': return { text: this.t('skillsPage.source.openclaw', 'OpenClaw'), cls: 'badge-info' };
        case 'bundled': return { text: this.t('skillsPage.source.bundled', 'Built-in'), cls: 'badge-success' };
        default: return { text: this.t('skillsPage.source.local', 'Local'), cls: 'badge-dim' };
      }
    },

    formatDownloads: function(n) {
      if (!n) return '0';
      if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
      if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
      return n.toString();
    },

    toolCountText(n) {
      return this.t('skillsPage.toolCount', '{count} tool(s)', { count: n || 0 });
    },

    promptContextText() {
      return this.t('skillsPage.promptContext', '(prompt context)');
    },

    searchLoadingText() {
      return this.clawhubSearch
        ? this.t('skillsPage.searching', 'Searching ClawHub...')
        : this.t('skillsPage.loading', 'Loading skills...');
    },

    resultsText() {
      return this.t('skillsPage.resultsFor', '{count} result(s) for "{query}"', {
        count: this.clawhubResults.length,
        query: this.clawhubSearch
      });
    },

    downloadsText(n) {
      return this.t('skillsPage.downloads', '{count} downloads', {
        count: this.formatDownloads(n)
      });
    },

    starsText(n) {
      return this.t('skillsPage.stars', '{count} stars', { count: n || 0 });
    },

    toolsAvailableText(n) {
      return this.t('skillsPage.toolsAvailable', '{count} tool(s) available', { count: n || 0 });
    },

    andMoreText(n) {
      return this.t('skillsPage.andMore', '... and {count} more', { count: n || 0 });
    },

    envText(values) {
      return this.t('skillsPage.envPrefix', 'Env: {value}', {
        value: Array.isArray(values) ? values.join(', ') : values
      });
    },

    quickStartName(qs) {
      return this.t('skillsPage.quickStartSkill.' + qs.name + '.name', qs.name);
    },

    quickStartDescription(qs) {
      return this.t('skillsPage.quickStartSkill.' + qs.name + '.description', qs.description);
    },

    async loadSkills() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await LibreFangAPI.get('/api/skills');
        this.skills = (data.skills || []).map(function(s) {
          return {
            name: s.name,
            description: s.description || '',
            version: s.version || '',
            author: s.author || '',
            runtime: s.runtime || 'unknown',
            tools_count: s.tools_count || 0,
            tags: s.tags || [],
            enabled: s.enabled !== false,
            source: s.source || { type: 'local' },
            has_prompt_context: !!s.has_prompt_context
          };
        });
      } catch(e) {
        this.skills = [];
        this.loadError = e.message || this.t('skillsPage.loadError', 'Could not load skills.');
      }
      this.loading = false;
    },

    async loadData() {
      await this.loadSkills();
    },

    // Debounced search — fires 350ms after user stops typing
    onSearchInput() {
      if (this._searchTimer) clearTimeout(this._searchTimer);
      var q = this.clawhubSearch.trim();
      if (!q) {
        this.clawhubResults = [];
        this.clawhubError = '';
        return;
      }
      var self = this;
      this._searchTimer = setTimeout(function() { self.searchClawHub(); }, 350);
    },

    // ClawHub search
    async searchClawHub() {
      if (!this.clawhubSearch.trim()) {
        this.clawhubResults = [];
        return;
      }
      this.clawhubLoading = true;
      this.clawhubError = '';
      try {
        var data = await LibreFangAPI.get('/api/clawhub/search?q=' + encodeURIComponent(this.clawhubSearch.trim()) + '&limit=20');
        this.clawhubResults = data.items || [];
        if (data.error) this.clawhubError = data.error;
      } catch(e) {
        this.clawhubResults = [];
        this.clawhubError = e.message || this.t('skillsPage.searchFailed', 'Search failed');
      }
      this.clawhubLoading = false;
    },

    // Clear search and go back to browse
    clearSearch() {
      this.clawhubSearch = '';
      this.clawhubResults = [];
      this.clawhubError = '';
      if (this._searchTimer) clearTimeout(this._searchTimer);
    },

    // ClawHub browse by sort (with 60s client-side cache)
    async browseClawHub(sort) {
      this.clawhubSort = sort || 'trending';
      var ckey = 'browse:' + this.clawhubSort;
      var cached = this._browseCache[ckey];
      if (cached && (Date.now() - cached.ts) < 60000) {
        this.clawhubBrowseResults = cached.data.items || [];
        this.clawhubNextCursor = cached.data.next_cursor || null;
        return;
      }
      this.clawhubLoading = true;
      this.clawhubError = '';
      this.clawhubNextCursor = null;
      try {
        var data = await LibreFangAPI.get('/api/clawhub/browse?sort=' + this.clawhubSort + '&limit=20');
        this.clawhubBrowseResults = data.items || [];
        this.clawhubNextCursor = data.next_cursor || null;
        if (data.error) this.clawhubError = data.error;
        this._browseCache[ckey] = { ts: Date.now(), data: data };
      } catch(e) {
        this.clawhubBrowseResults = [];
        this.clawhubError = e.message || this.t('skillsPage.browseFailed', 'Browse failed');
      }
      this.clawhubLoading = false;
    },

    // ClawHub load more results
    async loadMoreClawHub() {
      if (!this.clawhubNextCursor || this.clawhubLoading) return;
      this.clawhubLoading = true;
      try {
        var data = await LibreFangAPI.get('/api/clawhub/browse?sort=' + this.clawhubSort + '&limit=20&cursor=' + encodeURIComponent(this.clawhubNextCursor));
        this.clawhubBrowseResults = this.clawhubBrowseResults.concat(data.items || []);
        this.clawhubNextCursor = data.next_cursor || null;
      } catch(e) {
        // silently fail on load more
      }
      this.clawhubLoading = false;
    },

    // Show skill detail
    async showSkillDetail(slug) {
      this.detailLoading = true;
      this.skillDetail = null;
      this.installResult = null;
      try {
        var data = await LibreFangAPI.get('/api/clawhub/skill/' + encodeURIComponent(slug));
        this.skillDetail = data;
      } catch(e) {
        LibreFangToast.error(this.t('skillsPage.detailFailed', 'Failed to load skill details'));
      }
      this.detailLoading = false;
    },

    closeDetail() {
      this.skillDetail = null;
      this.installResult = null;
      this.showSkillCode = false;
      this.skillCode = '';
      this.skillCodeFilename = '';
    },

    async viewSkillCode(slug) {
      if (this.showSkillCode) {
        this.showSkillCode = false;
        return;
      }
      this.skillCodeLoading = true;
      try {
        var data = await LibreFangAPI.get('/api/clawhub/skill/' + encodeURIComponent(slug) + '/code');
        this.skillCode = data.code || '';
        this.skillCodeFilename = data.filename || 'source';
        this.showSkillCode = true;
      } catch(e) {
        LibreFangToast.error(this.t('skillsPage.codeFailed', 'Could not load skill source code'));
      }
      this.skillCodeLoading = false;
    },

    // Install from ClawHub
    async installFromClawHub(slug) {
      this.installingSlug = slug;
      this.installResult = null;
      try {
        var data = await LibreFangAPI.post('/api/clawhub/install', { slug: slug });
        this.installResult = data;
        if (data.warnings && data.warnings.length > 0) {
          LibreFangToast.success(this.t('skillsPage.installedWithWarnings', 'Skill "{name}" installed with {count} warning(s)', {
            name: data.name,
            count: data.warnings.length
          }));
        } else {
          LibreFangToast.success(this.t('skillsPage.installedSuccess', 'Skill "{name}" installed successfully', {
            name: data.name
          }));
        }
        // Update installed state in detail modal if open
        if (this.skillDetail && this.skillDetail.slug === slug) {
          this.skillDetail.installed = true;
        }
        await this.loadSkills();
      } catch(e) {
        var msg = e.message || this.t('skillsPage.installFailedShort', 'Install failed');
        if (msg.includes('already_installed')) {
          LibreFangToast.error(this.t('skillsPage.alreadyInstalledToast', 'Skill is already installed'));
        } else if (msg.includes('SecurityBlocked')) {
          LibreFangToast.error(this.t('skillsPage.securityBlocked', 'Skill blocked by security scan'));
        } else {
          LibreFangToast.error(this.t('skillsPage.installFailed', 'Install failed: {message}', {
            message: msg
          }));
        }
      }
      this.installingSlug = null;
    },

    // Uninstall
    uninstallSkill: function(name) {
      var self = this;
      LibreFangToast.confirm(
        this.t('skillsPage.uninstallTitle', 'Uninstall Skill'),
        this.t('skillsPage.uninstallConfirm', 'Uninstall skill "{name}"? This cannot be undone.', {
          name: name
        }),
        async function() {
        try {
          await LibreFangAPI.post('/api/skills/uninstall', { name: name });
          LibreFangToast.success(self.t('skillsPage.uninstalled', 'Skill "{name}" uninstalled', { name: name }));
          await self.loadSkills();
        } catch(e) {
          LibreFangToast.error(self.t('skillsPage.uninstallFailed', 'Failed to uninstall skill: {message}', {
            message: e.message || e
          }));
        }
      });
    },

    // Create prompt-only skill
    async createDemoSkill(skill) {
      try {
        await LibreFangAPI.post('/api/skills/create', {
          name: skill.name,
          description: skill.description,
          runtime: 'prompt_only',
          prompt_context: skill.prompt_context || skill.description
        });
        LibreFangToast.success(this.t('skillsPage.created', 'Skill "{name}" created', { name: skill.name }));
        this.tab = 'installed';
        await this.loadSkills();
      } catch(e) {
        LibreFangToast.error(this.t('skillsPage.createFailed', 'Failed to create skill: {message}', {
          message: e.message || e
        }));
      }
    },

    // Load MCP servers
    async loadMcpServers() {
      this.mcpLoading = true;
      try {
        var data = await LibreFangAPI.get('/api/mcp/servers');
        this.mcpServers = data;
      } catch(e) {
        this.mcpServers = { configured: [], connected: [], total_configured: 0, total_connected: 0 };
      }
      this.mcpLoading = false;
    },

    // Category search on ClawHub
    searchCategory: function(cat) {
      this.clawhubSearch = cat.name;
      this.searchClawHub();
    },

    // Quick start skills (prompt-only, zero deps)
    quickStartSkills: [
      { name: 'code-review-guide', description: 'Adds code review best practices and checklist to agent context.', prompt_context: 'You are an expert code reviewer. When reviewing code:\n1. Check for bugs and logic errors\n2. Evaluate code style and readability\n3. Look for security vulnerabilities\n4. Suggest performance improvements\n5. Verify error handling\n6. Check test coverage' },
      { name: 'writing-style', description: 'Configurable writing style guide for content generation.', prompt_context: 'Follow these writing guidelines:\n- Use clear, concise language\n- Prefer active voice over passive voice\n- Keep paragraphs short (3-4 sentences)\n- Use bullet points for lists\n- Maintain consistent tone throughout' },
      { name: 'api-design', description: 'REST API design patterns and conventions.', prompt_context: 'When designing REST APIs:\n- Use nouns for resources, not verbs\n- Use HTTP methods correctly (GET, POST, PUT, DELETE)\n- Return appropriate status codes\n- Use pagination for list endpoints\n- Version your API\n- Document all endpoints' },
      { name: 'security-checklist', description: 'OWASP-aligned security review checklist.', prompt_context: 'Security review checklist (OWASP aligned):\n- Input validation on all user inputs\n- Output encoding to prevent XSS\n- Parameterized queries to prevent SQL injection\n- Authentication and session management\n- Access control checks\n- CSRF protection\n- Security headers\n- Error handling without information leakage' },
    ],

    // Check if skill is installed by slug
    isSkillInstalled: function(slug) {
      return this.skills.some(function(s) {
        return s.source && s.source.type === 'clawhub' && s.source.slug === slug;
      });
    },

    // Check if skill is installed by name
    isSkillInstalledByName: function(name) {
      return this.skills.some(function(s) { return s.name === name; });
    },
  };
}
