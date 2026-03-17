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

    init: function() {
      this.loadStats();
      this.loadMemories();
    },

    loadStats: function() {
      var self = this;
      LibreFangAPI.request('GET', '/api/memory/stats').then(function(data) {
        self.stats = data;
      }).catch(function() {
        self.stats = {};
      });
    },

    loadMemories: function() {
      var self = this;
      self.loading = true;
      self.searchQuery = '';
      LibreFangAPI.request('GET', '/api/memory').then(function(data) {
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
      LibreFangAPI.request('GET', '/api/memory/search?q=' + encodeURIComponent(self.searchQuery) + '&limit=50').then(function(data) {
        self.memories = data.memories || [];
        self.loading = false;
      }).catch(function() {
        self.memories = [];
        self.loading = false;
      });
    },

    deleteMemory: function(id) {
      var self = this;
      if (!confirm('Delete this memory?')) return;
      LibreFangAPI.request('DELETE', '/api/memory/' + id).then(function() {
        self.memories = self.memories.filter(function(m) { return m.id !== id; });
        self.loadStats();
        LibreFangToast.success('Memory deleted');
      }).catch(function() {
        LibreFangToast.error('Failed to delete memory');
      });
    },

    viewHistory: function(id) {
      var self = this;
      LibreFangAPI.request('GET', '/api/memory/' + id + '/history').then(function(data) {
        self.historyItems = data.versions || [];
        self.showHistory = true;
      }).catch(function() {
        self.historyItems = [];
        self.showHistory = true;
      });
    }
  };
}
