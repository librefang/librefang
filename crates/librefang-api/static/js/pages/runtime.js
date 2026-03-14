// Runtime page — system overview and provider status
document.addEventListener('alpine:init', function() {
  Alpine.data('runtimePage', function() {
    return {
      _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
      loading: true,
      loadError: '',
      uptime: '-',
      agentCount: 0,
      version: '-',
      defaultModel: '-',
      platform: '-',
      arch: '-',
      apiListen: '-',
      homeDir: '-',
      logLevel: '-',
      networkEnabled: false,
      providers: [],

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

      formatUptimeShort(diff) {
        if (diff < 60) return this.t('runtimePage.secondsShort', '{count}s', { count: diff });
        if (diff < 3600) {
          return this.t('runtimePage.minutesSecondsShort', '{minutes}m {seconds}s', {
            minutes: Math.floor(diff / 60),
            seconds: diff % 60
          });
        }
        if (diff < 86400) {
          return this.t('runtimePage.hoursMinutesShort', '{hours}h {minutes}m', {
            hours: Math.floor(diff / 3600),
            minutes: Math.floor((diff % 3600) / 60)
          });
        }
        return this.t('runtimePage.daysHoursShort', '{days}d {hours}h', {
          days: Math.floor(diff / 86400),
          hours: Math.floor((diff % 86400) / 3600)
        });
      },

      networkStatusText() {
        return this.networkEnabled
          ? this.t('runtimePage.enabled', 'Enabled')
          : this.t('runtimePage.disabled', 'Disabled');
      },

      providerStatusText(provider) {
        if (provider.reachable) return this.t('runtimePage.online', 'Online');
        if (provider.auth_status === 'Configured' || provider.auth_status === 'configured') {
          return this.t('runtimePage.ready', 'Ready');
        }
        return this.t('runtimePage.notConfigured', 'Not configured');
      },

      latencyText(provider) {
        return provider.latency_ms ? provider.latency_ms + 'ms' : '-';
      },

      async loadData() {
        this.loading = true;
        this.loadError = '';
        try {
          var results = await Promise.all([
            LibreFangAPI.get('/api/status'),
            LibreFangAPI.get('/api/version'),
            LibreFangAPI.get('/api/providers'),
            LibreFangAPI.get('/api/agents')
          ]);
          var status = results[0];
          var ver = results[1];
          var prov = results[2];
          var agents = results[3];

          this.version = ver.version || '-';
          this.platform = ver.platform || '-';
          this.arch = ver.arch || '-';
          this.agentCount = agents && agents.total != null ? agents.total : (Array.isArray(agents) ? agents.length : 0);
          this.defaultModel = status.default_model || '-';
          this.apiListen = status.api_listen || status.listen || '-';
          this.homeDir = status.home_dir || '-';
          this.logLevel = status.log_level || '-';
          this.networkEnabled = !!status.network_enabled;

          // Compute uptime from uptime_seconds
          var diff = status.uptime_seconds || 0;
          this.uptime = this.formatUptimeShort(diff);

          this.providers = (prov.providers || []).filter(function(p) {
            return p.auth_status === 'Configured' || p.auth_status === 'configured' || p.reachable || p.is_local;
          }).sort(function(a, b) {
            return (a.auth_status === 'configured' ? 0 : 1) - (b.auth_status === 'configured' ? 0 : 1);
          });
        } catch(e) {
          this.loadError = e.message || this.t('runtimePage.loadError', 'Could not load runtime data.');
          console.error('Runtime load error:', e);
        }
        this.loading = false;
      }
    };
  });
});
