// LibreFang Sessions Page — Session listing + Memory tab
'use strict';

function sessionsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    tab: 'sessions',
    // -- Sessions state --
    sessions: [],
    searchFilter: '',
    loading: true,
    loadError: '',

    // -- Memory state --
    memAgentId: '',
    kvPairs: [],
    showAdd: false,
    newKey: '',
    newValue: '""',
    editingKey: null,
    editingValue: '',
    memLoading: false,
    memLoadError: '',

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
      var lang = this._currentLang;
      if (typeof i18n === 'undefined') return this.interpolate(fallback || key, params);
      var translated = i18n.t(key, params);
      if (!translated || translated.charAt(0) === '[') {
        return this.interpolate(fallback || key, params);
      }
      return translated;
    },

    // -- Sessions methods --
    async loadSessions() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await LibreFangAPI.get('/api/sessions');
        var sessions = data.sessions || [];
        var agents = Alpine.store('app').agents;
        var agentMap = {};
        agents.forEach(function(a) { agentMap[a.id] = a.name; });
        sessions.forEach(function(s) {
          s.agent_name = agentMap[s.agent_id] || '';
        });
        this.sessions = sessions;
      } catch(e) {
        this.sessions = [];
        this.loadError = e.message || this.t('sessionsPage.loadError', 'Could not load sessions.');
      }
      this.loading = false;
    },

    async loadData() { return this.loadSessions(); },

    get filteredSessions() {
      var f = this.searchFilter.toLowerCase();
      if (!f) return this.sessions;
      return this.sessions.filter(function(s) {
        return (s.agent_name || '').toLowerCase().indexOf(f) !== -1 ||
               (s.agent_id || '').toLowerCase().indexOf(f) !== -1;
      });
    },

    openInChat(session) {
      var agents = Alpine.store('app').agents;
      var agent = agents.find(function(a) { return a.id === session.agent_id; });
      if (agent) {
        Alpine.store('app').pendingAgent = agent;
      }
      location.hash = 'agents';
    },

    deleteSession(sessionId) {
      var self = this;
      LibreFangToast.confirm(
        this.t('sessionsPage.deleteSessionTitle', 'Delete Session'),
        this.t('sessionsPage.deleteSessionConfirm', 'This will permanently remove the session and its messages.'),
        async function() {
        try {
          await LibreFangAPI.del('/api/sessions/' + sessionId);
          self.sessions = self.sessions.filter(function(s) { return s.session_id !== sessionId; });
          LibreFangToast.success(self.t('sessionsPage.sessionDeleted', 'Session deleted'));
        } catch(e) {
          LibreFangToast.error(self.t('sessionsPage.deleteSessionFailed', 'Failed to delete session: {message}', { message: e.message }));
        }
      });
    },

    // -- Memory methods --
    async loadKv() {
      if (!this.memAgentId) { this.kvPairs = []; return; }
      this.memLoading = true;
      this.memLoadError = '';
      try {
        var data = await LibreFangAPI.get('/api/memory/agents/' + this.memAgentId + '/kv');
        this.kvPairs = data.kv_pairs || [];
      } catch(e) {
        this.kvPairs = [];
        this.memLoadError = e.message || this.t('sessionsPage.loadMemoryError', 'Could not load memory data.');
      }
      this.memLoading = false;
    },

    async addKey() {
      if (!this.memAgentId || !this.newKey.trim()) return;
      var keyName = this.newKey;
      var value;
      try { value = JSON.parse(this.newValue); } catch(e) { value = this.newValue; }
      try {
        await LibreFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.newKey), { value: value });
        this.showAdd = false;
        LibreFangToast.success(this.t('sessionsPage.keySaved', 'Key "{key}" saved', { key: keyName }));
        this.newKey = '';
        this.newValue = '""';
        await this.loadKv();
      } catch(e) {
        LibreFangToast.error(this.t('sessionsPage.saveKeyFailed', 'Failed to save key: {message}', { message: e.message }));
      }
    },

    deleteKey(key) {
      var self = this;
      LibreFangToast.confirm(
        this.t('sessionsPage.deleteKeyTitle', 'Delete Key'),
        this.t('sessionsPage.deleteKeyConfirm', 'Delete key "{key}"? This cannot be undone.', { key: key }),
        async function() {
        try {
          await LibreFangAPI.del('/api/memory/agents/' + self.memAgentId + '/kv/' + encodeURIComponent(key));
          LibreFangToast.success(self.t('sessionsPage.keyDeleted', 'Key "{key}" deleted', { key: key }));
          await self.loadKv();
        } catch(e) {
          LibreFangToast.error(self.t('sessionsPage.deleteKeyFailed', 'Failed to delete key: {message}', { message: e.message }));
        }
      });
    },

    startEdit(kv) {
      this.editingKey = kv.key;
      this.editingValue = typeof kv.value === 'object' ? JSON.stringify(kv.value, null, 2) : String(kv.value);
    },

    cancelEdit() {
      this.editingKey = null;
      this.editingValue = '';
    },

    async saveEdit() {
      if (!this.editingKey || !this.memAgentId) return;
      var value;
      try { value = JSON.parse(this.editingValue); } catch(e) { value = this.editingValue; }
      try {
        await LibreFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.editingKey), { value: value });
        LibreFangToast.success(this.t('sessionsPage.keyUpdated', 'Key "{key}" updated', { key: this.editingKey }));
        this.editingKey = null;
        this.editingValue = '';
        await this.loadKv();
      } catch(e) {
        LibreFangToast.error(this.t('sessionsPage.saveFailed', 'Failed to save: {message}', { message: e.message }));
      }
    },

    keyCountText() {
      return this.t('sessionsPage.keysCount', '{count} key(s)', { count: this.kvPairs.length });
    }
  };
}
