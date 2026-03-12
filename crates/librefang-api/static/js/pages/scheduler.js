// LibreFang Scheduler Page — Cron job management + event triggers unified view
'use strict';

function schedulerPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    tab: 'jobs',

    // -- Scheduled Jobs state --
    jobs: [],
    loading: true,
    loadError: '',

    // -- Event Triggers state --
    triggers: [],
    trigLoading: false,
    trigLoadError: '',

    // -- Run History state --
    history: [],
    historyLoading: false,

    // -- Create Job form --
    showCreateForm: false,
    newJob: {
      name: '',
      cron: '',
      agent_id: '',
      message: '',
      enabled: true
    },
    creating: false,

    // -- Run Now state --
    runningJobId: '',

    // Cron presets
    cronPresets: [
      { labelKey: 'everyMinute', label: 'Every minute', cron: '* * * * *' },
      { labelKey: 'every5Minutes', label: 'Every 5 minutes', cron: '*/5 * * * *' },
      { labelKey: 'every15Minutes', label: 'Every 15 minutes', cron: '*/15 * * * *' },
      { labelKey: 'every30Minutes', label: 'Every 30 minutes', cron: '*/30 * * * *' },
      { labelKey: 'everyHour', label: 'Every hour', cron: '0 * * * *' },
      { labelKey: 'every6Hours', label: 'Every 6 hours', cron: '0 */6 * * *' },
      { labelKey: 'dailyMidnight', label: 'Daily at midnight', cron: '0 0 * * *' },
      { labelKey: 'daily9am', label: 'Daily at 9am', cron: '0 9 * * *' },
      { labelKey: 'weekdays9am', label: 'Weekdays at 9am', cron: '0 9 * * 1-5' },
      { labelKey: 'everyMonday9am', label: 'Every Monday 9am', cron: '0 9 * * 1' },
      { labelKey: 'firstOfMonth', label: 'First of month', cron: '0 0 1 * *' }
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

    presetLabel(preset) {
      return this.t('schedulerPage.cron.' + preset.labelKey, preset.label);
    },

    activeJobsCountText() {
      return this.t('schedulerPage.activeJobsCount', '{active}/{total} active', {
        active: this.jobCount(),
        total: this.jobs.length
      });
    },

    triggerCountText() {
      return this.t('schedulerPage.triggersCount', '{count}', { count: this.triggers.length });
    },

    statusText(enabled) {
      return enabled
        ? this.t('schedulerPage.active', 'Active')
        : this.t('schedulerPage.paused', 'Paused');
    },

    historyTypeText(type) {
      return type === 'schedule'
        ? this.t('schedulerPage.cronJob', 'Cron Job')
        : this.t('schedulerPage.trigger', 'Trigger');
    },

    // ── Lifecycle ──

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        await this.loadJobs();
      } catch(e) {
        this.loadError = e.message || this.t('schedulerPage.loadError', 'Could not load scheduler data.');
      }
      this.loading = false;
    },

    async loadJobs() {
      var data = await LibreFangAPI.get('/api/cron/jobs');
      var raw = data.jobs || [];
      // Normalize cron API response to flat fields the UI expects
      this.jobs = raw.map(function(j) {
        var cron = '';
        if (j.schedule) {
          if (j.schedule.kind === 'cron') cron = j.schedule.expr || '';
          else if (j.schedule.kind === 'every') cron = 'every ' + j.schedule.every_secs + 's';
          else if (j.schedule.kind === 'at') cron = 'at ' + (j.schedule.at || '');
        }
        return {
          id: j.id,
          name: j.name,
          cron: cron,
          agent_id: j.agent_id,
          message: j.action ? j.action.message || '' : '',
          enabled: j.enabled,
          last_run: j.last_run,
          next_run: j.next_run,
          delivery: j.delivery ? j.delivery.kind || '' : '',
          created_at: j.created_at
        };
      });
    },

    async loadTriggers() {
      this.trigLoading = true;
      this.trigLoadError = '';
      try {
        var data = await LibreFangAPI.get('/api/triggers');
        this.triggers = Array.isArray(data) ? data : [];
      } catch(e) {
        this.triggers = [];
        this.trigLoadError = e.message || this.t('schedulerPage.loadTriggersError', 'Could not load triggers.');
      }
      this.trigLoading = false;
    },

    async loadHistory() {
      this.historyLoading = true;
      try {
        var historyItems = [];
        var jobs = this.jobs || [];
        for (var i = 0; i < jobs.length; i++) {
          var job = jobs[i];
          if (job.last_run) {
            historyItems.push({
              timestamp: job.last_run,
              name: job.name || this.t('schedulerPage.unnamed', '(unnamed)'),
              type: 'schedule',
              status: this.t('schedulerPage.completedStatus', 'completed'),
              run_count: 0
            });
          }
        }
        var triggers = this.triggers || [];
        for (var j = 0; j < triggers.length; j++) {
          var t = triggers[j];
          if (t.fire_count > 0) {
            historyItems.push({
              timestamp: t.created_at,
              name: this.t('schedulerPage.triggerPrefix', 'Trigger: {type}', {
                type: this.triggerType(t.pattern)
              }),
              type: 'trigger',
              status: this.t('schedulerPage.firedStatus', 'fired'),
              run_count: t.fire_count
            });
          }
        }
        historyItems.sort(function(a, b) {
          return new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime();
        });
        this.history = historyItems;
      } catch(e) {
        this.history = [];
      }
      this.historyLoading = false;
    },

    // ── Job CRUD ──

    async createJob() {
      if (!this.newJob.name.trim()) {
        LibreFangToast.warn(this.t('schedulerPage.enterJobName', 'Please enter a job name'));
        return;
      }
      if (!this.newJob.cron.trim()) {
        LibreFangToast.warn(this.t('schedulerPage.enterCron', 'Please enter a cron expression'));
        return;
      }
      this.creating = true;
      try {
        var jobName = this.newJob.name;
        var body = {
          agent_id: this.newJob.agent_id,
          name: this.newJob.name,
          schedule: { kind: 'cron', expr: this.newJob.cron },
          action: {
            kind: 'agent_turn',
            message: this.newJob.message || this.t('schedulerPage.defaultMessage', 'Scheduled task: {name}', {
              name: this.newJob.name
            })
          },
          delivery: { kind: 'last_channel' },
          enabled: this.newJob.enabled
        };
        await LibreFangAPI.post('/api/cron/jobs', body);
        this.showCreateForm = false;
        this.newJob = { name: '', cron: '', agent_id: '', message: '', enabled: true };
        LibreFangToast.success(this.t('schedulerPage.created', 'Schedule "{name}" created', { name: jobName }));
        await this.loadJobs();
      } catch(e) {
        LibreFangToast.error(this.t('schedulerPage.createFailed', 'Failed to create schedule: {message}', {
          message: e.message || e
        }));
      }
      this.creating = false;
    },

    async toggleJob(job) {
      try {
        var newState = !job.enabled;
        await LibreFangAPI.put('/api/cron/jobs/' + job.id + '/enable', { enabled: newState });
        job.enabled = newState;
        LibreFangToast.success(this.t(newState ? 'schedulerPage.enabledToast' : 'schedulerPage.pausedToast', newState ? 'Schedule enabled' : 'Schedule paused'));
      } catch(e) {
        LibreFangToast.error(this.t('schedulerPage.toggleFailed', 'Failed to toggle schedule: {message}', {
          message: e.message || e
        }));
      }
    },

    deleteJob(job) {
      var self = this;
      var jobName = job.name || job.id;
      LibreFangToast.confirm(
        this.t('schedulerPage.deleteScheduleTitle', 'Delete Schedule'),
        this.t('schedulerPage.deleteScheduleConfirm', 'Delete "{name}"? This cannot be undone.', { name: jobName }),
        async function() {
        try {
          await LibreFangAPI.del('/api/cron/jobs/' + job.id);
          self.jobs = self.jobs.filter(function(j) { return j.id !== job.id; });
          LibreFangToast.success(self.t('schedulerPage.deleted', 'Schedule "{name}" deleted', { name: jobName }));
        } catch(e) {
          LibreFangToast.error(self.t('schedulerPage.deleteFailed', 'Failed to delete schedule: {message}', {
            message: e.message || e
          }));
        }
      });
    },

    async runNow(job) {
      this.runningJobId = job.id;
      try {
        var result = await LibreFangAPI.post('/api/schedules/' + job.id + '/run', {});
        if (result.status === 'completed') {
          LibreFangToast.success(this.t('schedulerPage.runSuccess', 'Schedule "{name}" executed successfully', {
            name: job.name || this.t('schedulerPage.job', 'job')
          }));
          job.last_run = new Date().toISOString();
        } else {
          LibreFangToast.error(this.t('schedulerPage.runFailed', 'Schedule run failed: {message}', {
            message: result.error || this.t('schedulerPage.unknownError', 'Unknown error')
          }));
        }
      } catch(e) {
        LibreFangToast.error(this.t('schedulerPage.runUnavailable', 'Run Now is not yet available for cron jobs'));
      }
      this.runningJobId = '';
    },

    // ── Trigger helpers ──

    triggerType(pattern) {
      if (!pattern) return this.t('status.unknown', 'unknown');
      if (typeof pattern === 'string') return pattern;
      var keys = Object.keys(pattern);
      if (keys.length === 0) return this.t('status.unknown', 'unknown');
      var key = keys[0];
      var names = {
        lifecycle: this.t('schedulerPage.triggerType.lifecycle', 'Lifecycle'),
        agent_spawned: this.t('schedulerPage.triggerType.agentSpawned', 'Agent Spawned'),
        agent_terminated: this.t('schedulerPage.triggerType.agentTerminated', 'Agent Terminated'),
        system: this.t('schedulerPage.triggerType.system', 'System'),
        system_keyword: this.t('schedulerPage.triggerType.systemKeyword', 'System Keyword'),
        memory_update: this.t('schedulerPage.triggerType.memoryUpdate', 'Memory Update'),
        memory_key_pattern: this.t('schedulerPage.triggerType.memoryKey', 'Memory Key'),
        all: this.t('schedulerPage.triggerType.allEvents', 'All Events'),
        content_match: this.t('schedulerPage.triggerType.contentMatch', 'Content Match')
      };
      return names[key] || key.replace(/_/g, ' ');
    },

    async toggleTrigger(trigger) {
      try {
        var newState = !trigger.enabled;
        await LibreFangAPI.put('/api/triggers/' + trigger.id, { enabled: newState });
        trigger.enabled = newState;
        LibreFangToast.success(this.t(newState ? 'schedulerPage.triggerEnabled' : 'schedulerPage.triggerDisabled', newState ? 'Trigger enabled' : 'Trigger disabled'));
      } catch(e) {
        LibreFangToast.error(this.t('schedulerPage.toggleTriggerFailed', 'Failed to toggle trigger: {message}', {
          message: e.message || e
        }));
      }
    },

    deleteTrigger(trigger) {
      var self = this;
      LibreFangToast.confirm(
        this.t('schedulerPage.deleteTriggerTitle', 'Delete Trigger'),
        this.t('schedulerPage.deleteTriggerConfirm', 'Delete this trigger? This cannot be undone.'),
        async function() {
        try {
          await LibreFangAPI.del('/api/triggers/' + trigger.id);
          self.triggers = self.triggers.filter(function(t) { return t.id !== trigger.id; });
          LibreFangToast.success(self.t('schedulerPage.triggerDeleted', 'Trigger deleted'));
        } catch(e) {
          LibreFangToast.error(self.t('schedulerPage.deleteTriggerFailed', 'Failed to delete trigger: {message}', {
            message: e.message || e
          }));
        }
      });
    },

    // ── Utility ──

    get availableAgents() {
      return Alpine.store('app').agents || [];
    },

    agentName(agentId) {
      if (!agentId) return this.t('schedulerPage.anyAgent', '(any)');
      var agents = this.availableAgents;
      for (var i = 0; i < agents.length; i++) {
        if (agents[i].id === agentId) return agents[i].name;
      }
      if (agentId.length > 12) return agentId.substring(0, 8) + '...';
      return agentId;
    },

    describeCron(expr) {
      if (!expr) return '';
      // Handle non-cron schedule descriptions
      if (expr.indexOf('every ') === 0) return expr;
      if (expr.indexOf('at ') === 0) return this.t('schedulerPage.oneTimeAt', 'One-time: {time}', {
        time: expr.substring(3)
      });

      var map = {
        '* * * * *': this.t('schedulerPage.cron.everyMinute', 'Every minute'),
        '*/2 * * * *': this.t('schedulerPage.cron.every2Minutes', 'Every 2 minutes'),
        '*/5 * * * *': this.t('schedulerPage.cron.every5Minutes', 'Every 5 minutes'),
        '*/10 * * * *': this.t('schedulerPage.cron.every10Minutes', 'Every 10 minutes'),
        '*/15 * * * *': this.t('schedulerPage.cron.every15Minutes', 'Every 15 minutes'),
        '*/30 * * * *': this.t('schedulerPage.cron.every30Minutes', 'Every 30 minutes'),
        '0 * * * *': this.t('schedulerPage.cron.everyHour', 'Every hour'),
        '0 */2 * * *': this.t('schedulerPage.cron.every2Hours', 'Every 2 hours'),
        '0 */4 * * *': this.t('schedulerPage.cron.every4Hours', 'Every 4 hours'),
        '0 */6 * * *': this.t('schedulerPage.cron.every6Hours', 'Every 6 hours'),
        '0 */12 * * *': this.t('schedulerPage.cron.every12Hours', 'Every 12 hours'),
        '0 0 * * *': this.t('schedulerPage.cron.dailyMidnight', 'Daily at midnight'),
        '0 6 * * *': this.t('schedulerPage.cron.daily6am', 'Daily at 6:00 AM'),
        '0 9 * * *': this.t('schedulerPage.cron.daily9amLong', 'Daily at 9:00 AM'),
        '0 12 * * *': this.t('schedulerPage.cron.dailyNoon', 'Daily at noon'),
        '0 18 * * *': this.t('schedulerPage.cron.daily6pm', 'Daily at 6:00 PM'),
        '0 9 * * 1-5': this.t('schedulerPage.cron.weekdays9amLong', 'Weekdays at 9:00 AM'),
        '0 9 * * 1': this.t('schedulerPage.cron.mondays9am', 'Mondays at 9:00 AM'),
        '0 0 * * 0': this.t('schedulerPage.cron.sundaysMidnight', 'Sundays at midnight'),
        '0 0 1 * *': this.t('schedulerPage.cron.firstOfMonthLong', '1st of every month'),
        '0 0 * * 1': this.t('schedulerPage.cron.mondaysMidnight', 'Mondays at midnight')
      };
      if (map[expr]) return map[expr];

      var parts = expr.split(' ');
      if (parts.length !== 5) return expr;

      var min = parts[0];
      var hour = parts[1];
      var dom = parts[2];
      var mon = parts[3];
      var dow = parts[4];

      if (min.indexOf('*/') === 0 && hour === '*' && dom === '*' && mon === '*' && dow === '*') {
        return this.t('schedulerPage.everyMinutes', 'Every {count} minutes', { count: min.substring(2) });
      }
      if (min === '0' && hour.indexOf('*/') === 0 && dom === '*' && mon === '*' && dow === '*') {
        return this.t('schedulerPage.everyHours', 'Every {count} hours', { count: hour.substring(2) });
      }

      var dowNames = { '0': 'Sun', '1': 'Mon', '2': 'Tue', '3': 'Wed', '4': 'Thu', '5': 'Fri', '6': 'Sat', '7': 'Sun',
                       '1-5': 'Weekdays', '0,6': 'Weekends', '6,0': 'Weekends' };

      if (dom === '*' && mon === '*' && min.match(/^\d+$/) && hour.match(/^\d+$/)) {
        var h = parseInt(hour, 10);
        var m = parseInt(min, 10);
        var ampm = h >= 12 ? 'PM' : 'AM';
        var h12 = h === 0 ? 12 : (h > 12 ? h - 12 : h);
        var mStr = m < 10 ? '0' + m : '' + m;
        var timeStr = h12 + ':' + mStr + ' ' + ampm;
        if (dow === '*') return this.t('schedulerPage.dailyAt', 'Daily at {time}', { time: timeStr });
        var dowLabel = dowNames[dow] || this.t('schedulerPage.dayOfWeek', 'DoW {value}', { value: dow });
        return this.t('schedulerPage.dayAt', '{day} at {time}', { day: dowLabel, time: timeStr });
      }

      return expr;
    },

    applyCronPreset(preset) {
      this.newJob.cron = preset.cron;
    },

    formatTime(ts) {
      if (!ts) return '-';
      try {
        var d = new Date(ts);
        if (isNaN(d.getTime())) return '-';
        return d.toLocaleString();
      } catch(e) { return '-'; }
    },

    relativeTime(ts) {
      if (!ts) return this.t('schedulerPage.never', 'never');
      try {
        var diff = Date.now() - new Date(ts).getTime();
        if (isNaN(diff)) return this.t('schedulerPage.never', 'never');
        if (diff < 0) {
          // Future time
          var absDiff = Math.abs(diff);
          if (absDiff < 60000) return this.t('schedulerPage.inLessThanMinute', 'in <1m');
          if (absDiff < 3600000) return this.t('schedulerPage.inMinutes', 'in {count}m', { count: Math.floor(absDiff / 60000) });
          if (absDiff < 86400000) return this.t('schedulerPage.inHours', 'in {count}h', { count: Math.floor(absDiff / 3600000) });
          return this.t('schedulerPage.inDays', 'in {count}d', { count: Math.floor(absDiff / 86400000) });
        }
        if (diff < 60000) return this.t('time.now', 'just now');
        if (diff < 3600000) return this.t('time.minutesAgo', '{count}m ago', { count: Math.floor(diff / 60000) });
        if (diff < 86400000) return this.t('time.hoursAgo', '{count}h ago', { count: Math.floor(diff / 3600000) });
        return this.t('time.daysAgo', '{count}d ago', { count: Math.floor(diff / 86400000) });
      } catch(e) { return this.t('schedulerPage.never', 'never'); }
    },

    jobCount() {
      var enabled = 0;
      for (var i = 0; i < this.jobs.length; i++) {
        if (this.jobs[i].enabled) enabled++;
      }
      return enabled;
    },

    triggerCount() {
      var enabled = 0;
      for (var i = 0; i < this.triggers.length; i++) {
        if (this.triggers[i].enabled) enabled++;
      }
      return enabled;
    }
  };
}
