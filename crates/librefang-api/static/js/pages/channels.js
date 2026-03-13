// LibreFang Channels Page — OpenClaw-style setup UX with QR code support
'use strict';

function channelsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    allChannels: [],
    categoryFilter: 'all',
    searchQuery: '',
    setupModal: null,
    configuring: false,
    testing: {},
    formValues: {},
    showAdvanced: false,
    showBusinessApi: false,
    loading: true,
    loadError: '',
    pollTimer: null,

    // Setup flow step tracking
    setupStep: 1, // 1=Configure, 2=Verify, 3=Ready
    testPassed: false,

    // WhatsApp QR state
    qr: {
      loading: false,
      available: false,
      dataUrl: '',
      sessionId: '',
      message: '',
      help: '',
      connected: false,
      expired: false,
      error: ''
    },
    qrPollTimer: null,

    categories: [
      { key: 'all', label: 'All' },
      { key: 'messaging', label: 'Messaging' },
      { key: 'social', label: 'Social' },
      { key: 'enterprise', label: 'Enterprise' },
      { key: 'developer', label: 'Developer' },
      { key: 'notifications', label: 'Notifications' }
    ],

    init() {
      var self = this;
      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
      });
      // Read URL params: #channels?cat=messaging&q=search
      var hashParts = window.location.hash.split('?');
      if (hashParts.length > 1) {
        var params = new URLSearchParams(hashParts[1]);
        if (params.get('cat')) self.categoryFilter = params.get('cat');
        if (params.get('q')) self.searchQuery = params.get('q');
      }
      // Watch category and search changes → update URL
      this.$watch('categoryFilter', function(val) { self._updateURL(); });
      this.$watch('searchQuery', function(val) { self._updateURL(); });
    },

    _updateURL() {
      var params = [];
      if (this.categoryFilter && this.categoryFilter !== 'all') params.push('cat=' + encodeURIComponent(this.categoryFilter));
      if (this.searchQuery) params.push('q=' + encodeURIComponent(this.searchQuery));
      var hash = 'channels' + (params.length ? '?' + params.join('&') : '');
      if (window.location.hash !== '#' + hash) {
        history.replaceState(null, '', '#' + hash);
      }
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
      return this.t('channelsPage.category.' + cat.key, cat.label);
    },

    categoryCountText(cat) {
      return this.t('channelsPage.categoryCount', '{label} ({count})', {
        label: this.categoryLabel(cat),
        count: this.categoryCount(cat.key)
      });
    },

    configuredSummaryText() {
      return this.t('channelsPage.configuredSummary', '{configured}/{total} configured', {
        configured: this.configuredCount,
        total: this.allChannels.length
      });
    },

    setupButtonText(channel) {
      return channel.configured
        ? this.t('btn.edit', 'Edit')
        : this.t('channelsPage.setUp', 'Set up');
    },

    saveButtonText() {
      if (this.configuring) return this.t('channelsPage.saving', 'Saving...');
      return this.setupModal && this.setupModal.configured
        ? this.t('channelsPage.update', 'Update')
        : this.t('channelsPage.saveAndTest', 'Save & Test');
    },

    testButtonText() {
      if (!this.setupModal) return this.t('btn.test', 'Test');
      return this.testing[this.setupModal.name]
        ? this.t('channelsPage.testing', 'Testing...')
        : this.t('btn.test', 'Test');
    },

    get filteredChannels() {
      var self = this;
      return this.allChannels.filter(function(ch) {
        if (self.categoryFilter !== 'all' && ch.category !== self.categoryFilter) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          return ch.name.toLowerCase().indexOf(q) !== -1 ||
                 ch.display_name.toLowerCase().indexOf(q) !== -1 ||
                 ch.description.toLowerCase().indexOf(q) !== -1;
        }
        return true;
      });
    },

    get configuredCount() {
      return this.allChannels.filter(function(ch) { return ch.configured; }).length;
    },

    categoryCount(cat) {
      var all = this.allChannels.filter(function(ch) { return cat === 'all' || ch.category === cat; });
      var configured = all.filter(function(ch) { return ch.configured; });
      return configured.length + '/' + all.length;
    },

    basicFields() {
      if (!this.setupModal || !this.setupModal.fields) return [];
      return this.setupModal.fields.filter(function(f) { return !f.advanced; });
    },

    advancedFields() {
      if (!this.setupModal || !this.setupModal.fields) return [];
      return this.setupModal.fields.filter(function(f) { return f.advanced; });
    },

    hasAdvanced() {
      return this.advancedFields().length > 0;
    },

    isQrChannel() {
      return this.setupModal && this.setupModal.setup_type === 'qr';
    },

    async loadChannels() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await LibreFangAPI.get('/api/channels');
        this.allChannels = (data.channels || []).map(function(ch) {
          ch.connected = ch.configured && ch.has_token;
          return ch;
        }).sort(function(a, b) {
          return (a.configured ? 0 : 1) - (b.configured ? 0 : 1);
        });
      } catch(e) {
        this.loadError = e.message || this.t('channelsPage.loadError', 'Could not load channels.');
      }
      this.loading = false;
      this.startPolling();
    },

    async loadData() { return this.loadChannels(); },

    startPolling() {
      var self = this;
      if (this.pollTimer) clearInterval(this.pollTimer);
      this.pollTimer = setInterval(function() { self.refreshStatus(); }, 15000);
    },

    async refreshStatus() {
      try {
        var data = await LibreFangAPI.get('/api/channels');
        var byName = {};
        (data.channels || []).forEach(function(ch) { byName[ch.name] = ch; });
        this.allChannels.forEach(function(c) {
          var fresh = byName[c.name];
          if (fresh) {
            c.configured = fresh.configured;
            c.has_token = fresh.has_token;
            c.connected = fresh.configured && fresh.has_token;
            c.fields = fresh.fields;
          }
        });
      } catch(e) { console.warn('Channel refresh failed:', e.message); }
    },

    statusBadge(ch) {
      if (!ch.configured) return { text: this.t('channelsPage.status.notConfigured', 'Not Configured'), cls: 'badge-muted' };
      if (!ch.has_token) return { text: this.t('channelsPage.status.missingToken', 'Missing Token'), cls: 'badge-warn' };
      if (ch.connected) return { text: this.t('channelsPage.status.ready', 'Ready'), cls: 'badge-success' };
      return { text: this.t('channelsPage.status.configured', 'Configured'), cls: 'badge-info' };
    },

    difficultyClass(d) {
      if (d === 'Easy') return 'difficulty-easy';
      if (d === 'Hard') return 'difficulty-hard';
      return 'difficulty-medium';
    },

    openSetup(ch) {
      this.setupModal = ch;
      // Pre-populate form values from saved config (non-secret fields).
      var vals = {};
      if (ch.fields) {
        ch.fields.forEach(function(f) {
          if (f.value !== undefined && f.value !== null && f.type !== 'secret') {
            vals[f.key] = String(f.value);
          }
        });
      }
      this.formValues = vals;
      this.showAdvanced = false;
      this.showBusinessApi = false;
      this.setupStep = ch.configured ? 3 : 1;
      this.testPassed = !!ch.configured;
      this.resetQR();
      // Auto-start QR flow for QR-type channels
      if (ch.setup_type === 'qr') {
        this.startQR();
      }
    },

    // ── QR Code Flow (WhatsApp Web style) ──────────────────────────

    resetQR() {
      this.qr = {
        loading: false, available: false, dataUrl: '', sessionId: '',
        message: '', help: '', connected: false, expired: false, error: ''
      };
      if (this.qrPollTimer) { clearInterval(this.qrPollTimer); this.qrPollTimer = null; }
    },

    async startQR() {
      this.qr.loading = true;
      this.qr.error = '';
      this.qr.connected = false;
      this.qr.expired = false;
      try {
        var result = await LibreFangAPI.post('/api/channels/whatsapp/qr/start', {});
        this.qr.available = result.available || false;
        this.qr.dataUrl = result.qr_data_url || '';
        this.qr.sessionId = result.session_id || '';
        this.qr.message = result.message || '';
        this.qr.help = result.help || '';
        this.qr.connected = result.connected || false;
        if (this.qr.available && this.qr.dataUrl && !this.qr.connected) {
          this.pollQR();
        }
        if (this.qr.connected) {
          LibreFangToast.success(this.t('channelsPage.whatsAppConnected', 'WhatsApp connected!'));
          await this.refreshStatus();
        }
      } catch(e) {
        this.qr.error = e.message || this.t('channelsPage.qrStartFailed', 'Could not start QR login');
      }
      this.qr.loading = false;
    },

    pollQR() {
      var self = this;
      if (this.qrPollTimer) clearInterval(this.qrPollTimer);
      this.qrPollTimer = setInterval(async function() {
        try {
          var result = await LibreFangAPI.get('/api/channels/whatsapp/qr/status?session_id=' + encodeURIComponent(self.qr.sessionId));
          if (result.connected) {
            clearInterval(self.qrPollTimer);
            self.qrPollTimer = null;
            self.qr.connected = true;
            self.qr.message = result.message || self.t('channelsPage.connected', 'Connected!');
            LibreFangToast.success(self.t('channelsPage.whatsAppLinked', 'WhatsApp linked successfully!'));
            await self.refreshStatus();
          } else if (result.expired) {
            clearInterval(self.qrPollTimer);
            self.qrPollTimer = null;
            self.qr.expired = true;
            self.qr.message = self.t('channelsPage.qrExpired', 'QR code expired. Click to generate a new one.');
          } else {
            self.qr.message = result.message || self.t('channelsPage.waitingForScan', 'Waiting for scan...');
          }
        } catch(e) { /* silent retry */ }
      }, 3000);
    },

    // ── Standard Form Flow ─────────────────────────────────────────

    async saveChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;
      this.configuring = true;
      try {
        await LibreFangAPI.post('/api/channels/' + name + '/configure', {
          fields: this.formValues
        });
        this.setupStep = 2;
        // Auto-test after save
        try {
          var testResult = await LibreFangAPI.post('/api/channels/' + name + '/test', {});
          if (testResult.status === 'ok') {
            this.testPassed = true;
            this.setupStep = 3;
            LibreFangToast.success(this.t('channelsPage.activated', '{name} activated!', {
              name: this.setupModal.display_name
            }));
          } else {
            LibreFangToast.success(this.t('channelsPage.savedWithMessage', '{name} saved. {message}', {
              name: this.setupModal.display_name,
              message: testResult.message || ''
            }));
          }
        } catch(te) {
          LibreFangToast.success(this.t('channelsPage.savedTestToVerify', '{name} saved. Test to verify connection.', {
            name: this.setupModal.display_name
          }));
        }
        await this.refreshStatus();
      } catch(e) {
        LibreFangToast.error(this.t('channelsPage.failedMessage', 'Failed: {message}', {
          message: e.message || this.t('channelsPage.unknownError', 'Unknown error')
        }));
      }
      this.configuring = false;
    },

    async removeChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;
      var displayName = this.setupModal.display_name;
      var self = this;
      LibreFangToast.confirm(
        this.t('channelsPage.removeTitle', 'Remove Channel'),
        this.t('channelsPage.removeConfirm', 'Remove {name} configuration? This will deactivate the channel.', {
          name: displayName
        }),
        async function() {
        try {
          await LibreFangAPI.delete('/api/channels/' + name + '/configure');
          LibreFangToast.success(self.t('channelsPage.removed', '{name} removed and deactivated.', {
            name: displayName
          }));
          await self.refreshStatus();
          self.setupModal = null;
        } catch(e) {
          LibreFangToast.error(self.t('channelsPage.failedMessage', 'Failed: {message}', {
            message: e.message || self.t('channelsPage.unknownError', 'Unknown error')
          }));
        }
      });
    },

    async testChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;
      this.testing[name] = true;
      try {
        var result = await LibreFangAPI.post('/api/channels/' + name + '/test', {});
        if (result.status === 'ok') {
          this.testPassed = true;
          this.setupStep = 3;
          LibreFangToast.success(result.message);
        } else {
          LibreFangToast.error(result.message);
        }
      } catch(e) {
        LibreFangToast.error(this.t('channelsPage.testFailed', 'Test failed: {message}', {
          message: e.message || this.t('channelsPage.unknownError', 'Unknown error')
        }));
      }
      this.testing[name] = false;
    },

    async copyConfig(ch) {
      var tpl = ch ? ch.config_template : (this.setupModal ? this.setupModal.config_template : '');
      if (!tpl) return;
      try {
        await navigator.clipboard.writeText(tpl);
        LibreFangToast.success(this.t('channelsPage.copied', 'Copied to clipboard'));
      } catch(e) {
        LibreFangToast.error(this.t('channelsPage.copyFailed', 'Copy failed'));
      }
    },

    destroy() {
      if (this.pollTimer) { clearInterval(this.pollTimer); this.pollTimer = null; }
      if (this.qrPollTimer) { clearInterval(this.qrPollTimer); this.qrPollTimer = null; }
    }
  };
}
