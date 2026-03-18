// LibreFang Memory Page — Proactive memory management (mem0-style)
'use strict';

function memoryPage() {
  return {
    memories: [],
    stats: {},
    searchQuery: '',
    loading: false,
    showHistory: false,
    historyItems: [],

    // Agent filter state
    agents: [],
    selectedAgentId: '',

    // Manual memory creation form
    showAddForm: false,
    addForm: { content: '', level: 'user' },
    addingMemory: false,

    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    t: function(key, fallback, params) {
      if (typeof i18n === 'undefined') return this._interpolate(fallback || key, params);
      var translated = i18n.t(key, params);
      if (!translated || translated.charAt(0) === '[') {
        return this._interpolate(fallback || key, params);
      }
      return translated;
    },
    _interpolate: function(text, params) {
      if (!params || typeof text !== 'string') return text;
      return text.replace(/\{(\w+)\}/g, function(match, key) {
        return params[key] !== undefined ? params[key] : match;
      });
    },

    init: function() {
      var self = this;
      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
      });
      this.loadAgents();
      this.loadStats();
      this.loadMemories();
    },

    loadAgents: function() {
      var self = this;
      LibreFangAPI.request('GET', '/api/agents').then(function(data) {
        self.agents = data || [];
      }).catch(function() {
        self.agents = [];
      });
    },

    get statsEndpoint() {
      if (this.selectedAgentId) {
        return '/api/memory/agents/' + encodeURIComponent(this.selectedAgentId) + '/stats';
      }
      return '/api/memory/stats';
    },

    get searchEndpoint() {
      if (this.selectedAgentId) {
        return '/api/memory/agents/' + encodeURIComponent(this.selectedAgentId) + '/search';
      }
      return '/api/memory/search';
    },

    get listEndpoint() {
      if (this.selectedAgentId) {
        return '/api/memory/agents/' + encodeURIComponent(this.selectedAgentId) + '/search?q=&limit=50';
      }
      return '/api/memory';
    },

    onAgentFilterChange: function() {
      this.loadStats();
      this.loadMemories();
    },

    loadStats: function() {
      var self = this;
      LibreFangAPI.request('GET', this.statsEndpoint).then(function(data) {
        self.stats = data;
      }).catch(function() {
        self.stats = {};
      });
    },

    loadMemories: function() {
      var self = this;
      self.loading = true;
      self.searchQuery = '';
      LibreFangAPI.request('GET', this.listEndpoint).then(function(data) {
        self.memories = data.memories || [];
        self.loading = false;
      }).catch(function() {
        self.memories = [];
        self.loading = false;
      });
    },

    searchMemories: function() {
      var self = this;
      if (!self.searchQuery.trim()) {
        self.loadMemories();
        return;
      }
      self.loading = true;
      var url = this.searchEndpoint + '?q=' + encodeURIComponent(self.searchQuery) + '&limit=50';
      LibreFangAPI.request('GET', url).then(function(data) {
        self.memories = data.memories || [];
        self.loading = false;
      }).catch(function() {
        self.memories = [];
        self.loading = false;
      });
    },

    deleteMemory: function(id) {
      var self = this;
      if (!confirm(this.t('memoryPage.confirmDelete', 'Delete this memory?'))) return;
      LibreFangAPI.request('DELETE', '/api/memory/items/' + id).then(function() {
        self.memories = self.memories.filter(function(m) { return m.id !== id; });
        self.loadStats();
        LibreFangToast.success(self.t('memoryPage.memoryDeleted', 'Memory deleted'));
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.deleteMemoryFailed', 'Failed to delete memory'));
      });
    },

    viewHistory: function(id) {
      var self = this;
      LibreFangAPI.request('GET', '/api/memory/items/' + id + '/history').then(function(data) {
        self.historyItems = data.versions || [];
        self.showHistory = true;
      }).catch(function() {
        self.historyItems = [];
        self.showHistory = true;
      });
    },

    toggleAddForm: function() {
      this.showAddForm = !this.showAddForm;
      if (this.showAddForm) {
        this.addForm = { content: '', level: 'user' };
      }
    },

    addMemory: function() {
      var self = this;
      if (!self.addForm.content.trim()) {
        LibreFangToast.warn(self.t('memoryPage.contentRequired', 'Please enter memory content'));
        return;
      }
      self.addingMemory = true;
      var body = {
        messages: [{ role: 'user', content: self.addForm.content.trim() }]
      };
      if (self.selectedAgentId) {
        body.agent_id = self.selectedAgentId;
      }
      LibreFangAPI.request('POST', '/api/memory', body).then(function(data) {
        var count = data.added || 0;
        LibreFangToast.success(self.t('memoryPage.memoryAdded', '{count} memory/memories added', { count: count }));
        self.addForm = { content: '', level: 'user' };
        self.showAddForm = false;
        self.addingMemory = false;
        self.loadStats();
        self.loadMemories();
      }).catch(function(e) {
        LibreFangToast.error(self.t('memoryPage.addMemoryFailed', 'Failed to add memory: {message}', { message: e.message }));
        self.addingMemory = false;
      });
    }
  };
}
