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

    // Pagination
    pageOffset: 0,
    pageLimit: 20,
    totalMemories: 0,

    // Agent filter state
    agents: [],
    selectedAgentId: '',

    // Bulk selection
    selectedIds: [],

    // Knowledge graph relations
    showRelations: false,
    relations: [],
    relationSource: '',
    relationTarget: '',

    // Manual memory creation form
    showAddForm: false,
    addForm: { content: '' },
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
        self.agents = Array.isArray(data) ? data : (data && data.items ? data.items : []);
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
        return '/api/memory/agents/' + encodeURIComponent(this.selectedAgentId) + '?offset=' + this.pageOffset + '&limit=' + this.pageLimit;
      }
      return '/api/memory?offset=' + this.pageOffset + '&limit=' + this.pageLimit;
    },

    get totalPages() {
      return Math.max(1, Math.ceil(this.totalMemories / this.pageLimit));
    },

    get currentPage() {
      return Math.floor(this.pageOffset / this.pageLimit) + 1;
    },

    onAgentFilterChange: function() {
      this.pageOffset = 0;
      this.relations = [];
      this.showRelations = false;
      this.loadStats();
      this.loadMemories();
    },

    goToPage: function(page) {
      if (page < 1 || page > this.totalPages) return;
      this.pageOffset = (page - 1) * this.pageLimit;
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
      self.selectedIds = [];
      LibreFangAPI.request('GET', this.listEndpoint).then(function(data) {
        self.memories = data.memories || [];
        self.totalMemories = data.total || self.memories.length;
        // Clamp offset if total shrunk (e.g. memories deleted by another tab)
        if (self.pageOffset >= self.totalMemories && self.totalMemories > 0) {
          self.pageOffset = Math.max(0, (Math.ceil(self.totalMemories / self.pageLimit) - 1) * self.pageLimit);
        }
        self.loading = false;
      }).catch(function() {
        self.memories = [];
        self.totalMemories = 0;
        self.pageOffset = 0;
        self.loading = false;
      });
    },

    searchMemories: function() {
      var self = this;
      if (!self.searchQuery.trim()) {
        self.pageOffset = 0;
        self.loadMemories();
        return;
      }
      self.loading = true;
      var url = this.searchEndpoint + '?q=' + encodeURIComponent(self.searchQuery) + '&limit=50';
      LibreFangAPI.request('GET', url).then(function(data) {
        self.memories = data.memories || [];
        self.totalMemories = self.memories.length;
        self.loading = false;
      }).catch(function() {
        self.memories = [];
        self.totalMemories = 0;
        self.loading = false;
      });
    },

    startEdit: function(mem) {
      mem._editing = true;
      mem._editContent = mem.content;
    },

    cancelEdit: function(mem) {
      mem._editing = false;
      delete mem._editContent;
    },

    saveEdit: function(mem) {
      var self = this;
      if (!mem._editContent || !mem._editContent.trim()) {
        LibreFangToast.warn(self.t('memoryPage.contentRequired', 'Please enter memory content'));
        return;
      }
      LibreFangAPI.request('PUT', '/api/memory/items/' + mem.id, { content: mem._editContent.trim() }).then(function() {
        mem.content = mem._editContent.trim();
        mem._editing = false;
        delete mem._editContent;
        LibreFangToast.success(self.t('memoryPage.memoryUpdated', 'Memory updated'));
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.updateFailed', 'Failed to update memory'));
      });
    },

    deleteMemory: function(id) {
      var self = this;
      if (!confirm(this.t('memoryPage.confirmDelete', 'Delete this memory?'))) return;
      LibreFangAPI.request('DELETE', '/api/memory/items/' + id).then(function() {
        self.memories = self.memories.filter(function(m) { return m.id !== id; });
        self.totalMemories = Math.max(0, self.totalMemories - 1);
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
        this.addForm = { content: '' };
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
        self.addForm = { content: '' };
        self.showAddForm = false;
        self.addingMemory = false;
        self.loadStats();
        self.loadMemories();
      }).catch(function(e) {
        LibreFangToast.error(self.t('memoryPage.addMemoryFailed', 'Failed to add memory: {message}', { message: e.message }));
        self.addingMemory = false;
      });
    },

    consolidateAgent: function() {
      var self = this;
      if (!self.selectedAgentId) {
        LibreFangToast.warn(self.t('memoryPage.selectAgentFirst', 'Select an agent first'));
        return;
      }
      if (!confirm(self.t('memoryPage.confirmConsolidate', 'Merge duplicate memories for this agent?'))) return;
      LibreFangAPI.request('POST', '/api/memory/agents/' + self.selectedAgentId + '/consolidate').then(function() {
        LibreFangToast.success(self.t('memoryPage.consolidated', 'Memories consolidated'));
        self.loadStats();
        self.loadMemories();
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.consolidateFailed', 'Failed to consolidate memories'));
      });
    },

    cleanupExpired: function() {
      var self = this;
      if (!confirm(self.t('memoryPage.confirmCleanup', 'Remove expired session memories?'))) return;
      LibreFangAPI.request('POST', '/api/memory/cleanup').then(function(data) {
        var removed = data.removed || 0;
        LibreFangToast.success(self.t('memoryPage.cleanedUp', '{count} expired memories removed', { count: removed }));
        self.loadStats();
        self.loadMemories();
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.cleanupFailed', 'Failed to cleanup'));
      });
    },

    toggleSelect: function(id) {
      var idx = this.selectedIds.indexOf(id);
      if (idx === -1) { this.selectedIds.push(id); } else { this.selectedIds.splice(idx, 1); }
    },

    selectAll: function() {
      if (this.selectedIds.length === this.memories.length) {
        this.selectedIds = [];
      } else {
        this.selectedIds = this.memories.map(function(m) { return m.id; });
      }
    },

    bulkDelete: function() {
      var self = this;
      if (!self.selectedIds.length) return;
      if (!confirm(self.t('memoryPage.confirmBulkDelete', 'Delete {count} selected memories?', { count: self.selectedIds.length }))) return;
      LibreFangAPI.request('POST', '/api/memory/bulk-delete', { ids: self.selectedIds }).then(function(data) {
        LibreFangToast.success(self.t('memoryPage.bulkDeleted', '{count} memories deleted', { count: data.deleted || 0 }));
        self.selectedIds = [];
        self.loadStats();
        self.loadMemories();
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.bulkDeleteFailed', 'Failed to delete selected memories'));
      });
    },

    decayConfidence: function() {
      var self = this;
      if (!confirm(self.t('memoryPage.confirmDecay', 'Apply confidence decay to old memories?'))) return;
      LibreFangAPI.request('POST', '/api/memory/decay').then(function(data) {
        LibreFangToast.success(self.t('memoryPage.decayDone', 'Confidence decay applied (rate: {rate})', { rate: data.decay_rate || 0 }));
        self.loadStats();
        self.loadMemories();
      }).catch(function() {
        LibreFangToast.error(self.t('memoryPage.decayFailed', 'Failed to apply confidence decay'));
      });
    },

    loadRelations: function() {
      var self = this;
      if (!self.selectedAgentId) return;
      var url = '/api/memory/agents/' + encodeURIComponent(self.selectedAgentId) + '/relations';
      var params = [];
      if (self.relationSource.trim()) params.push('source=' + encodeURIComponent(self.relationSource.trim()));
      if (self.relationTarget.trim()) params.push('target=' + encodeURIComponent(self.relationTarget.trim()));
      if (params.length) url += '?' + params.join('&');
      LibreFangAPI.request('GET', url).then(function(data) {
        self.relations = data.matches || [];
      }).catch(function() {
        self.relations = [];
        LibreFangToast.error(self.t('memoryPage.relationsFailed', 'Failed to load relations'));
      });
    }
  };
}
