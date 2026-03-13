// LibreFang Comms Page — Agent topology & inter-agent communication feed
'use strict';

function commsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    topology: { nodes: [], edges: [] },
    events: [],
    loading: true,
    loadError: '',
    sseSource: null,
    showSendModal: false,
    showTaskModal: false,
    sendFrom: '',
    sendTo: '',
    sendMsg: '',
    sendLoading: false,
    taskTitle: '',
    taskDesc: '',
    taskAssign: '',
    taskLoading: false,

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

    topologyCountText() {
      return this.t('commsPage.agentsCount', '{count} agents', {
        count: this.topology.nodes.length
      });
    },

    eventsCountText() {
      return this.t('commsPage.eventsCount', '{count} events', {
        count: this.events.length
      });
    },

    selectAgentLabel() {
      return this.t('commsPage.selectAgent', 'Select agent...');
    },

    stateLabel(state) {
      return this.t('commsPage.state.' + String(state || '').toLowerCase(), state);
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var results = await Promise.all([
          LibreFangAPI.get('/api/comms/topology'),
          LibreFangAPI.get('/api/comms/events?limit=200')
        ]);
        this.topology = results[0] || { nodes: [], edges: [] };
        this.events = results[1] || [];
        this.startSSE();
      } catch(e) {
        this.loadError = e.message || this.t('commsPage.loadError', 'Could not load comms data.');
      }
      this.loading = false;
    },

    startSSE() {
      if (this.sseSource) this.sseSource.close();
      var self = this;
      var url = LibreFangAPI.baseUrl + '/api/comms/events/stream';
      if (LibreFangAPI.apiKey) url += '?token=' + encodeURIComponent(LibreFangAPI.apiKey);
      this.sseSource = new EventSource(url);
      this.sseSource.onmessage = function(ev) {
        if (ev.data === 'ping') return;
        try {
          var event = JSON.parse(ev.data);
          self.events.unshift(event);
          if (self.events.length > 200) self.events.length = 200;
          // Refresh topology on spawn/terminate events
          if (event.kind === 'agent_spawned' || event.kind === 'agent_terminated') {
            self.refreshTopology();
          }
        } catch(e) { /* ignore parse errors */ }
      };
    },

    stopSSE() {
      if (this.sseSource) {
        this.sseSource.close();
        this.sseSource = null;
      }
    },

    async refreshTopology() {
      try {
        this.topology = await LibreFangAPI.get('/api/comms/topology');
      } catch(e) { /* silent */ }
    },

    rootNodes() {
      var childIds = {};
      var self = this;
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'parent_child') childIds[e.to] = true;
      });
      return this.topology.nodes.filter(function(n) { return !childIds[n.id]; });
    },

    childrenOf(id) {
      var childIds = {};
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'parent_child' && e.from === id) childIds[e.to] = true;
      });
      return this.topology.nodes.filter(function(n) { return childIds[n.id]; });
    },

    peersOf(id) {
      var peerIds = {};
      this.topology.edges.forEach(function(e) {
        if (e.kind === 'peer') {
          if (e.from === id) peerIds[e.to] = true;
          if (e.to === id) peerIds[e.from] = true;
        }
      });
      return this.topology.nodes.filter(function(n) { return peerIds[n.id]; });
    },

    stateBadgeClass(state) {
      switch(state) {
        case 'Running': return 'badge badge-success';
        case 'Suspended': return 'badge badge-warning';
        case 'Terminated': case 'Crashed': return 'badge badge-danger';
        default: return 'badge badge-dim';
      }
    },

    eventBadgeClass(kind) {
      switch(kind) {
        case 'agent_message': return 'badge badge-info';
        case 'agent_spawned': return 'badge badge-success';
        case 'agent_terminated': return 'badge badge-danger';
        case 'task_posted': return 'badge badge-warning';
        case 'task_claimed': return 'badge badge-info';
        case 'task_completed': return 'badge badge-success';
        default: return 'badge badge-dim';
      }
    },

    eventIcon(kind) {
      switch(kind) {
        case 'agent_message': return '\u2709';
        case 'agent_spawned': return '+';
        case 'agent_terminated': return '\u2715';
        case 'task_posted': return '\u2691';
        case 'task_claimed': return '\u2690';
        case 'task_completed': return '\u2713';
        default: return '\u2022';
      }
    },

    eventLabel(kind) {
      switch(kind) {
        case 'agent_message': return this.t('commsPage.event.message', 'Message');
        case 'agent_spawned': return this.t('commsPage.event.spawned', 'Spawned');
        case 'agent_terminated': return this.t('commsPage.event.terminated', 'Terminated');
        case 'task_posted': return this.t('commsPage.event.taskPosted', 'Task Posted');
        case 'task_claimed': return this.t('commsPage.event.taskClaimed', 'Task Claimed');
        case 'task_completed': return this.t('commsPage.event.taskDone', 'Task Done');
        default: return kind;
      }
    },

    timeAgo(dateStr) {
      if (!dateStr) return '';
      var d = new Date(dateStr);
      var secs = Math.floor((Date.now() - d.getTime()) / 1000);
      if (secs < 5) return this.t('time.now', 'just now');
      if (secs < 60) return this.t('time.secondsAgo', '{count}s ago', { count: secs });
      if (secs < 3600) return this.t('time.minutesAgo', '{count}m ago', { count: Math.floor(secs / 60) });
      if (secs < 86400) return this.t('time.hoursAgo', '{count}h ago', { count: Math.floor(secs / 3600) });
      return this.t('time.daysAgo', '{count}d ago', { count: Math.floor(secs / 86400) });
    },

    openSendModal() {
      this.sendFrom = '';
      this.sendTo = '';
      this.sendMsg = '';
      this.showSendModal = true;
    },

    async submitSend() {
      if (!this.sendFrom || !this.sendTo || !this.sendMsg.trim()) return;
      this.sendLoading = true;
      try {
        await LibreFangAPI.post('/api/comms/send', {
          from_agent_id: this.sendFrom,
          to_agent_id: this.sendTo,
          message: this.sendMsg
        });
        LibreFangToast.success(this.t('commsPage.messageSent', 'Message sent'));
        this.showSendModal = false;
      } catch(e) {
        LibreFangToast.error(e.message || this.t('commsPage.sendFailed', 'Send failed'));
      }
      this.sendLoading = false;
    },

    openTaskModal() {
      this.taskTitle = '';
      this.taskDesc = '';
      this.taskAssign = '';
      this.showTaskModal = true;
    },

    async submitTask() {
      if (!this.taskTitle.trim()) return;
      this.taskLoading = true;
      try {
        var body = { title: this.taskTitle, description: this.taskDesc };
        if (this.taskAssign) body.assigned_to = this.taskAssign;
        await LibreFangAPI.post('/api/comms/task', body);
        LibreFangToast.success(this.t('commsPage.taskPosted', 'Task posted'));
        this.showTaskModal = false;
      } catch(e) {
        LibreFangToast.error(e.message || this.t('commsPage.taskFailed', 'Task failed'));
      }
      this.taskLoading = false;
    }
  };
}
