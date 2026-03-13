// LibreFang Approvals Page — Execution approval queue for sensitive agent actions
'use strict';

function approvalsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    approvals: [],
    filterStatus: 'all',
    loading: true,
    loadError: '',

    _updateURL() {
      var params = [];
      if (this.filterStatus && this.filterStatus !== 'all') params.push('filter=' + encodeURIComponent(this.filterStatus));
      var hash = 'approvals' + (params.length ? '?' + params.join('&') : '');
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
        if (params.get('filter')) self.filterStatus = params.get('filter');
      }
      this.$watch('filterStatus', function() { self._updateURL(); });
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

    get filtered() {
      var f = this.filterStatus;
      if (f === 'all') return this.approvals;
      return this.approvals.filter(function(a) { return a.status === f; });
    },

    get pendingCount() {
      return this.approvals.filter(function(a) { return a.status === 'pending'; }).length;
    },

    pendingCountText() {
      return this.t('approvals.pendingCount', '{count} pending', { count: this.pendingCount });
    },

    statusLabel(status) {
      var fallback = status ? status.charAt(0).toUpperCase() + status.slice(1) : this.t('status.unknown', 'unknown');
      return this.t('approvals.status.' + status, fallback);
    },

    actionLabel(action) {
      if (!action) return this.t('overview.actionUnknown', 'Unknown');
      var map = {
        AgentSpawn: 'overview.actionAgentCreated',
        AgentKill: 'overview.actionAgentStopped',
        AgentTerminated: 'overview.actionAgentStopped',
        ToolInvoke: 'overview.actionToolUsed',
        ToolResult: 'overview.actionToolCompleted',
        AgentMessage: 'overview.actionMessageIn',
        NetworkAccess: 'overview.actionNetworkAccess',
        ShellExec: 'overview.actionShellCommand',
        FileAccess: 'overview.actionFileAccess',
        MemoryAccess: 'overview.actionMemoryAccess',
        AuthAttempt: 'overview.actionLoginAttempt',
        AuthSuccess: 'overview.actionLoginOk',
        AuthFailure: 'overview.actionLoginFailed',
        CapabilityDenied: 'overview.actionDenied',
        RateLimited: 'overview.actionRateLimited'
      };
      return this.t(map[action] || '', action.replace(/([A-Z])/g, ' $1').trim());
    },

    agentMetaText(approval) {
      return this.t('approvals.agentMeta', 'Agent: {agent} - {time}', {
        agent: approval.agent_name || '-',
        time: this.timeAgo(approval.created_at)
      });
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await LibreFangAPI.get('/api/approvals');
        this.approvals = data.approvals || [];
      } catch(e) {
        this.loadError = e.message || this.t('approvals.loadError', 'Could not load approvals.');
      }
      this.loading = false;
    },

    async approve(id) {
      try {
        await LibreFangAPI.post('/api/approvals/' + id + '/approve', {});
        LibreFangToast.success(this.t('approvals.approvedToast', 'Approved'));
        await this.loadData();
      } catch(e) {
        LibreFangToast.error(e.message);
      }
    },

    async reject(id) {
      var self = this;
      LibreFangToast.confirm(
        self.t('approvals.rejectTitle', 'Reject Action'),
        self.t('approvals.rejectConfirm', 'Are you sure you want to reject this action?'),
        async function() {
        try {
          await LibreFangAPI.post('/api/approvals/' + id + '/reject', {});
          LibreFangToast.success(self.t('approvals.rejectedToast', 'Rejected'));
          await self.loadData();
        } catch(e) {
          LibreFangToast.error(e.message);
        }
      });
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
    }
  };
}
