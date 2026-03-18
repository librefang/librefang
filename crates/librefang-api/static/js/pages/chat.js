// LibreFang Chat Page — Agent chat with markdown + streaming
'use strict';

function chatPage() {
  var msgId = 0;
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    currentAgent: null,
    messages: [],
    inputText: '',
    sending: false,
    messageQueue: [],    // Queue for messages sent while streaming
    thinkingMode: 'off', // 'off' | 'on' | 'stream'
    _wsAgent: null,
    showSlashMenu: false,
    slashFilter: '',
    slashIdx: 0,
    attachments: [],
    dragOver: false,
    contextPressure: 'low', // green/yellow/orange/red indicator
    _typingTimeout: null,
    // Multi-session state
    sessions: [],
    sessionsOpen: false,
    searchOpen: false,
    searchQuery: '',
    // Voice recording state
    recording: false,
    _mediaRecorder: null,
    _audioChunks: [],
    recordingTime: 0,
    _recordingTimer: null,
    // Model autocomplete state
    showModelPicker: false,
    modelPickerList: [],
    modelPickerFilter: '',
    modelPickerIdx: 0,
    // Model switcher dropdown
    showModelSwitcher: false,
    modelSwitcherFilter: '',
    modelSwitcherProviderFilter: '',
    modelSwitcherIdx: 0,
    modelSwitching: false,
    _modelCache: null,
    _modelCacheTime: 0,
    slashCommands: [],
    _defaultSlashCommands: [
      { cmd: '/help', descKey: 'agentChat.cmd.help', descFallback: 'Show available commands' },
      { cmd: '/agents', descKey: 'agentChat.cmd.agents', descFallback: 'Switch to Agents page' },
      { cmd: '/new', descKey: 'agentChat.cmd.new', descFallback: 'Reset session (clear history)' },
      { cmd: '/compact', descKey: 'agentChat.cmd.compact', descFallback: 'Trigger LLM session compaction' },
      { cmd: '/model', descKey: 'agentChat.cmd.model', descFallback: 'Show or switch model (/model [name])' },
      { cmd: '/stop', descKey: 'agentChat.cmd.stop', descFallback: 'Cancel current agent run' },
      { cmd: '/usage', descKey: 'agentChat.cmd.usage', descFallback: 'Show session token usage & cost' },
      { cmd: '/think', descKey: 'agentChat.cmd.think', descFallback: 'Toggle extended thinking (/think [on|off|stream])' },
      { cmd: '/context', descKey: 'agentChat.cmd.context', descFallback: 'Show context window usage & pressure' },
      { cmd: '/verbose', descKey: 'agentChat.cmd.verbose', descFallback: 'Cycle tool detail level (/verbose [off|on|full])' },
      { cmd: '/queue', descKey: 'agentChat.cmd.queue', descFallback: 'Check if agent is processing' },
      { cmd: '/status', descKey: 'agentChat.cmd.status', descFallback: 'Show system status' },
      { cmd: '/clear', descKey: 'agentChat.cmd.clear', descFallback: 'Clear chat display' },
      { cmd: '/exit', descKey: 'agentChat.cmd.exit', descFallback: 'Disconnect from agent' },
      { cmd: '/budget', descKey: 'agentChat.cmd.budget', descFallback: 'Show spending limits and current costs' },
      { cmd: '/peers', descKey: 'agentChat.cmd.peers', descFallback: 'Show OFP peer network status' },
      { cmd: '/a2a', descKey: 'agentChat.cmd.a2a', descFallback: 'List discovered external A2A agents' }
    ],
    tokenCount: 0,

    // ── Tip Bar ──
    tipIndex: 0,
    tipKeys: [
      ['agentChat.tipCommands', 'Type / for commands'],
      ['agentChat.tipThinking', '/think on for reasoning'],
      ['agentChat.tipFocus', 'Ctrl+Shift+F for focus mode'],
      ['agentChat.tipAttach', 'Drag files to attach'],
      ['agentChat.tipModel', '/model to switch models'],
      ['agentChat.tipContext', '/context to check usage'],
      ['agentChat.tipVerbose', '/verbose off to hide tool details']
    ],
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
    get tips() {
      var self = this;
      return this.tipKeys.map(function(pair) {
        return self.t(pair[0], pair[1]);
      });
    },
    tipTimer: null,
    get currentTip() {
      if (localStorage.getItem('of-tips-off') === 'true') return '';
      return this.tips[this.tipIndex % this.tips.length];
    },
    dismissTips: function() { localStorage.setItem('of-tips-off', 'true'); },
    startTipCycle: function() {
      var self = this;
      if (this.tipTimer) clearInterval(this.tipTimer);
      this.tipTimer = setInterval(function() {
        self.tipIndex = (self.tipIndex + 1) % self.tips.length;
      }, 30000);
    },

    // Backward compat helper
    get thinkingEnabled() { return this.thinkingMode !== 'off'; },

    // Context pressure dot color
    get contextDotColor() {
      switch (this.contextPressure) {
        case 'critical': return '#ef4444';
        case 'high': return '#f97316';
        case 'medium': return '#eab308';
        default: return '#22c55e';
      }
    },

    get modelDisplayName() {
      if (!this.currentAgent) return '';
      var name = this.currentAgent.model_name || '';
      var short = name.replace(/-\d{8}$/, '');
      return short.length > 24 ? short.substring(0, 22) + '\u2026' : short;
    },

    get switcherProviders() {
      var seen = {};
      (this._modelCache || []).forEach(function(m) { seen[m.provider] = true; });
      return Object.keys(seen).sort();
    },

    get filteredSwitcherModels() {
      var models = this._modelCache || [];
      var provFilter = this.modelSwitcherProviderFilter;
      var textFilter = this.modelSwitcherFilter ? this.modelSwitcherFilter.toLowerCase() : '';
      if (!provFilter && !textFilter) return models;
      return models.filter(function(m) {
        if (provFilter && m.provider !== provFilter) return false;
        if (textFilter) {
          return m.id.toLowerCase().indexOf(textFilter) !== -1 ||
                 (m.display_name || '').toLowerCase().indexOf(textFilter) !== -1 ||
                 m.provider.toLowerCase().indexOf(textFilter) !== -1;
        }
        return true;
      });
    },

    get groupedSwitcherModels() {
      var filtered = this.filteredSwitcherModels;
      var groups = {}, order = [];
      filtered.forEach(function(m) {
        if (!groups[m.provider]) { groups[m.provider] = []; order.push(m.provider); }
        groups[m.provider].push(m);
      });
      return order.map(function(p) {
        return { provider: p.charAt(0).toUpperCase() + p.slice(1), models: groups[p] };
      });
    },

    queueText(count, withPlus) {
      return this.t(
        withPlus ? 'agentChat.queueCount' : 'agentChat.queueCountCompact',
        withPlus ? '+{count} queued' : '{count} queued',
        { count: count }
      );
    },

    sessionLabel(session) {
      if (session.label) return session.label;
      return this.t('agentChat.sessionDefault', 'Session {id}', {
        id: session.session_id.substring(0, 8)
      });
    },

    messageCountText(count) {
      return this.t('agentChat.messagesCount', '{count} messages', { count: count });
    },

    searchResultsText() {
      return this.t('agentChat.searchResults', '{filtered} of {total}', {
        filtered: this.filteredMessages.length,
        total: this.messages.length
      });
    },

    copyMessageTitle(msg) {
      return msg._copied
        ? this.t('agentChat.copied', 'Copied!')
        : this.t('agentChat.copyMessage', 'Copy message');
    },

    composerPlaceholder() {
      return this.recording
        ? this.t('agentChat.recordingRelease', 'Recording... release to send')
        : this.t('agentChat.messageLibreFang', 'Message LibreFang... (/ for commands)');
    },

    toolStatusText(tool) {
      if (tool.running) return this.t('agentChat.running', 'running...');
      if (tool.is_error) return this.t('status.error', 'error');
      if (tool.result && tool.result.length > 500) return Math.round(tool.result.length / 1024) + 'KB';
      return this.t('agentChat.done', 'done');
    },

    toolCharsText(result) {
      return this.t('agentChat.chars', '({count} chars)', { count: result.length });
    },

    audioLabel(tool) {
      return this.t('agentChat.audio', 'Audio: {name}', {
        name: tool._audioFile.split('/').pop()
      });
    },

    audioDurationText(ms) {
      return '~' + this.t('agentChat.secondsShort', '{count}s', {
        count: Math.round((ms || 0) / 1000)
      });
    },

    // Memory indicator helpers
    memoryToolCount(tools) {
      if (!tools || !tools.length) return 0;
      return tools.filter(function(t) {
        var n = (t.name || '').toLowerCase();
        return n === 'memory_recall' || n === 'memory_store' || n === 'memory_search' || n === 'memory_save';
      }).length;
    },

    memoryTools(tools) {
      if (!tools || !tools.length) return [];
      return tools.filter(function(t) {
        var n = (t.name || '').toLowerCase();
        return n === 'memory_recall' || n === 'memory_store' || n === 'memory_search' || n === 'memory_save';
      });
    },

    memoryIndicatorText(tools) {
      var count = this.memoryToolCount(tools);
      var recalled = 0;
      var saved = 0;
      (tools || []).forEach(function(t) {
        var n = (t.name || '').toLowerCase();
        if (n === 'memory_recall' || n === 'memory_search') recalled++;
        if (n === 'memory_store' || n === 'memory_save') saved++;
      });
      var parts = [];
      if (recalled > 0) parts.push(recalled + ' ' + this.t('memoryPage.memoriesUsed', 'memories used'));
      if (saved > 0) parts.push(saved + ' ' + this.t('memoryPage.memoriesSaved', 'memories saved'));
      return parts.join(', ') || (count + ' ' + this.t('memoryPage.memoriesUsed', 'memories used'));
    },

    footerStatusText() {
      if (this.tokenCount > 0) {
        return '~' + this.tokenCount + ' ' + this.t('agentChat.tokens', 'tokens');
      }
      if (this.attachments.length) {
        return this.t('agentChat.filesCount', '{count} file(s)', {
          count: this.attachments.length
        });
      }
      return '';
    },

    init() {
      var self = this;

      window.addEventListener('i18n-changed', function(event) {
        self._currentLang = event.detail.language;
        self._buildSlashCommands();
      });

      // Build localized slash commands from defaults
      this._buildSlashCommands();

      // Start tip cycle
      this.startTipCycle();

      // Fetch dynamic commands from server
      this.fetchCommands();

      // Ctrl+/ keyboard shortcut
      document.addEventListener('keydown', function(e) {
        if ((e.ctrlKey || e.metaKey) && e.key === '/') {
          e.preventDefault();
          var input = document.getElementById('msg-input');
          if (input) { input.focus(); self.inputText = '/'; }
        }
        // Ctrl+M for model switcher
        if ((e.ctrlKey || e.metaKey) && e.key === 'm' && self.currentAgent) {
          e.preventDefault();
          self.toggleModelSwitcher();
        }
        // Ctrl+F for chat search
        if ((e.ctrlKey || e.metaKey) && e.key === 'f' && self.currentAgent) {
          e.preventDefault();
          self.toggleSearch();
        }
      });

      // Load session + session list when agent changes
      this.$watch('currentAgent', function(agent) {
        if (agent) {
          self.loadSession(agent.id);
          self.loadSessions(agent.id);
        }
      });

      // Check for pending agent from Agents page (set before chat mounted)
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        self.selectAgent(store.pendingAgent);
        store.pendingAgent = null;
      }

      // Watch for future pending agent selections (e.g., user clicks agent while on chat)
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.selectAgent(agent);
          Alpine.store('app').pendingAgent = null;
        }
      });

      // Watch for slash commands + model autocomplete
      this.$watch('inputText', function(val) {
        var modelMatch = val.match(/^\/model\s+(.*)$/i);
        if (modelMatch) {
          self.showSlashMenu = false;
          self.modelPickerFilter = modelMatch[1].toLowerCase();
          if (!self.modelPickerList.length) {
            LibreFangAPI.get('/api/models').then(function(data) {
              self.modelPickerList = (data.models || []).filter(function(m) { return m.available; });
              self.showModelPicker = true;
              self.modelPickerIdx = 0;
            }).catch(function() {});
          } else {
            self.showModelPicker = true;
          }
        } else if (val.startsWith('/')) {
          self.showModelPicker = false;
          self.slashFilter = val.slice(1).toLowerCase();
          self.showSlashMenu = true;
          self.slashIdx = 0;
        } else {
          self.showSlashMenu = false;
          self.showModelPicker = false;
        }
      });
    },

    get filteredModelPicker() {
      if (!this.modelPickerFilter) return this.modelPickerList.slice(0, 15);
      var f = this.modelPickerFilter;
      return this.modelPickerList.filter(function(m) {
        return m.id.toLowerCase().indexOf(f) !== -1 || (m.display_name || '').toLowerCase().indexOf(f) !== -1 || m.provider.toLowerCase().indexOf(f) !== -1;
      }).slice(0, 15);
    },

    pickModel(modelId) {
      this.showModelPicker = false;
      this.inputText = '/model ' + modelId;
      this.sendMessage();
    },

    toggleModelSwitcher() {
      if (this.showModelSwitcher) { this.showModelSwitcher = false; return; }
      var self = this;
      var now = Date.now();
      if (this._modelCache && (now - this._modelCacheTime) < 300000) {
        this.modelSwitcherFilter = '';
        this.modelSwitcherProviderFilter = '';
        this.modelSwitcherIdx = 0;
        this.showModelSwitcher = true;
        this.$nextTick(function() {
          var el = document.getElementById('model-switcher-search');
          if (el) el.focus();
        });
        return;
      }
      LibreFangAPI.get('/api/models').then(function(data) {
        var models = (data.models || []).filter(function(m) { return m.available; });
        self._modelCache = models;
        self._modelCacheTime = Date.now();
        self.modelPickerList = models;
        self.modelSwitcherFilter = '';
        self.modelSwitcherProviderFilter = '';
        self.modelSwitcherIdx = 0;
        self.showModelSwitcher = true;
        self.$nextTick(function() {
          var el = document.getElementById('model-switcher-search');
          if (el) el.focus();
        });
      }).catch(function(e) {
        LibreFangToast.error(self.t('agentChat.loadModelsFailed', 'Failed to load models: {message}', { message: e.message }));
      });
    },

    switchModel(model) {
      if (!this.currentAgent) return;
      if (model.id === this.currentAgent.model_name) { this.showModelSwitcher = false; return; }
      var self = this;
      this.modelSwitching = true;
      LibreFangAPI.put('/api/agents/' + this.currentAgent.id + '/model', { model: model.id }).then(function(resp) {
        // Use server-resolved model/provider to stay in sync (fixes #387/#466)
        self.currentAgent.model_name = (resp && resp.model) || model.id;
        self.currentAgent.model_provider = (resp && resp.provider) || model.provider;
        LibreFangToast.success(self.t('agentChat.toast.modelSwitched', 'Switched to {model}', { model: model.display_name || model.id }));
        self.showModelSwitcher = false;
        self.modelSwitching = false;
      }).catch(function(e) {
        LibreFangToast.error(self.t('agentChat.toast.switchFailed', 'Switch failed: {message}', { message: e.message }));
        self.modelSwitching = false;
      });
    },

    _buildSlashCommands: function() {
      var self = this;
      this.slashCommands = this._defaultSlashCommands.map(function(c) {
        return { cmd: c.cmd, desc: self.t(c.descKey, c.descFallback) };
      });
    },

    // Fetch dynamic slash commands from server
    fetchCommands: function() {
      var self = this;
      LibreFangAPI.get('/api/commands').then(function(data) {
        if (data.commands && data.commands.length) {
          // Build a set of known cmds to avoid duplicates
          var existing = {};
          self.slashCommands.forEach(function(c) { existing[c.cmd] = true; });
          data.commands.forEach(function(c) {
            if (!existing[c.cmd]) {
              self.slashCommands.push({ cmd: c.cmd, desc: c.desc || '', source: c.source || 'server' });
              existing[c.cmd] = true;
            }
          });
        }
      }).catch(function() { /* silent — use hardcoded list */ });
    },

    get filteredSlashCommands() {
      if (!this.slashFilter) return this.slashCommands;
      var f = this.slashFilter;
      return this.slashCommands.filter(function(c) {
        return c.cmd.toLowerCase().indexOf(f) !== -1 || c.desc.toLowerCase().indexOf(f) !== -1;
      });
    },

    // Clear any stuck typing indicator after 120s
    _resetTypingTimeout: function() {
      var self = this;
      if (self._typingTimeout) clearTimeout(self._typingTimeout);
      self._typingTimeout = setTimeout(function() {
        // Auto-clear stuck typing indicators
        self.messages = self.messages.filter(function(m) { return !m.thinking; });
        self.sending = false;
      }, 120000);
    },

    _clearTypingTimeout: function() {
      if (this._typingTimeout) {
        clearTimeout(this._typingTimeout);
        this._typingTimeout = null;
      }
    },

    executeSlashCommand(cmd, cmdArgs) {
      this.showSlashMenu = false;
      this.inputText = '';
      var self = this;
      cmdArgs = cmdArgs || '';
      switch (cmd) {
        case '/help':
          self.messages.push({ id: ++msgId, role: 'system', text: self.slashCommands.map(function(c) { return '`' + c.cmd + '` — ' + c.desc; }).join('\n'), meta: '', tools: [] });
          self.scrollToBottom();
          break;
        case '/agents':
          location.hash = 'agents';
          break;
        case '/new':
          if (self.currentAgent) {
            LibreFangAPI.post('/api/agents/' + self.currentAgent.id + '/session/reset', {}).then(function() {
              self.messages = [];
              LibreFangToast.success(self.t('agentChat.toast.sessionReset', 'Session reset'));
            }).catch(function(e) { LibreFangToast.error(self.t('agentChat.toast.resetFailed', 'Reset failed: {message}', { message: e.message })); });
          }
          break;
        case '/compact':
          if (self.currentAgent) {
            self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.compacting', 'Compacting session...'), meta: '', tools: [] });
            LibreFangAPI.post('/api/agents/' + self.currentAgent.id + '/session/compact', {}).then(function(res) {
              self.messages.push({ id: ++msgId, role: 'system', text: res.message || self.t('agentChat.sys.compactDone', 'Compaction complete'), meta: '', tools: [] });
              self.scrollToBottom();
            }).catch(function(e) { LibreFangToast.error(self.t('agentChat.toast.compactFailed', 'Compaction failed: {message}', { message: e.message })); });
          }
          break;
        case '/stop':
          if (self.currentAgent) {
            LibreFangAPI.post('/api/agents/' + self.currentAgent.id + '/stop', {}).then(function(res) {
              self.messages.push({ id: ++msgId, role: 'system', text: res.message || self.t('agentChat.sys.runCancelled', 'Run cancelled'), meta: '', tools: [] });
              self.sending = false;
              self.scrollToBottom();
            }).catch(function(e) { LibreFangToast.error(self.t('agentChat.toast.stopFailed', 'Stop failed: {message}', { message: e.message })); });
          }
          break;
        case '/usage':
          if (self.currentAgent) {
            var approxTokens = self.messages.reduce(function(sum, m) { return sum + Math.round((m.text || '').length / 4); }, 0);
            self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.sessionUsageTitle', 'Session Usage') + '**\n- ' + self.t('agentChat.sys.messages', 'Messages') + ': ' + self.messages.length + '\n- ' + self.t('agentChat.sys.approxTokens', 'Approx tokens') + ': ~' + approxTokens, meta: '', tools: [] });
            self.scrollToBottom();
          }
          break;
        case '/think':
          if (cmdArgs === 'on') {
            self.thinkingMode = 'on';
          } else if (cmdArgs === 'off') {
            self.thinkingMode = 'off';
          } else if (cmdArgs === 'stream') {
            self.thinkingMode = 'stream';
          } else {
            // Cycle: off -> on -> stream -> off
            if (self.thinkingMode === 'off') self.thinkingMode = 'on';
            else if (self.thinkingMode === 'on') self.thinkingMode = 'stream';
            else self.thinkingMode = 'off';
          }
          var modeLabel = self.thinkingMode === 'stream' ? self.t('agentChat.sys.thinkStream', 'enabled (streaming reasoning)') : (self.thinkingMode === 'on' ? self.t('agentChat.sys.thinkOn', 'enabled') : self.t('agentChat.sys.thinkOff', 'disabled'));
          self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.thinkStatus', 'Extended thinking') + ' **' + modeLabel + '**. ' +
            (self.thinkingMode === 'stream' ? self.t('agentChat.sys.thinkStreamDesc', 'Reasoning tokens will appear in a collapsible panel.') :
             self.thinkingMode === 'on' ? self.t('agentChat.sys.thinkOnDesc', 'The agent will show its reasoning when supported by the model.') :
             self.t('agentChat.sys.thinkOffDesc', 'Normal response mode.')), meta: '', tools: [] });
          self.scrollToBottom();
          break;
        case '/context':
          // Send via WS command
          if (self.currentAgent && LibreFangAPI.isWsConnected()) {
            LibreFangAPI.wsSend({ type: 'command', command: 'context', args: '' });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.notConnected', 'Not connected. Connect to an agent first.'), meta: '', tools: [] });
            self.scrollToBottom();
          }
          break;
        case '/verbose':
          if (self.currentAgent && LibreFangAPI.isWsConnected()) {
            LibreFangAPI.wsSend({ type: 'command', command: 'verbose', args: cmdArgs });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.notConnected', 'Not connected. Connect to an agent first.'), meta: '', tools: [] });
            self.scrollToBottom();
          }
          break;
        case '/queue':
          if (self.currentAgent && LibreFangAPI.isWsConnected()) {
            LibreFangAPI.wsSend({ type: 'command', command: 'queue', args: '' });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.notConnectedShort', 'Not connected.'), meta: '', tools: [] });
            self.scrollToBottom();
          }
          break;
        case '/status':
          LibreFangAPI.get('/api/status').then(function(s) {
            self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.systemStatus', 'System Status') + '**\n- ' + self.t('agentChat.sys.agents', 'Agents') + ': ' + (s.agent_count || 0) + '\n- ' + self.t('agentChat.sys.uptime', 'Uptime') + ': ' + (s.uptime_seconds || 0) + 's\n- ' + self.t('agentChat.sys.version', 'Version') + ': ' + (s.version || '?'), meta: '', tools: [] });
            self.scrollToBottom();
          }).catch(function() {});
          break;
        case '/model':
          if (self.currentAgent) {
            if (cmdArgs) {
              LibreFangAPI.put('/api/agents/' + self.currentAgent.id + '/model', { model: cmdArgs }).then(function(resp) {
                // Use server-resolved model/provider (fixes #387/#466)
                var resolvedModel = (resp && resp.model) || cmdArgs;
                var resolvedProvider = (resp && resp.provider) || '';
                self.currentAgent.model_name = resolvedModel;
                if (resolvedProvider) { self.currentAgent.model_provider = resolvedProvider; }
                self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.modelSwitchedTo', 'Model switched to: `{model}`', { model: resolvedModel }) + (resolvedProvider ? ' (' + self.t('agentChat.sys.provider', 'provider') + ': `' + resolvedProvider + '`)' : ''), meta: '', tools: [] });
                self.scrollToBottom();
              }).catch(function(e) { LibreFangToast.error(self.t('agentChat.toast.modelSwitchFailed', 'Model switch failed: {message}', { message: e.message })); });
            } else {
              self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.currentModel', 'Current Model') + '**\n- ' + self.t('agentChat.sys.providerLabel', 'Provider') + ': `' + (self.currentAgent.model_provider || '?') + '`\n- ' + self.t('agentChat.sys.modelLabel', 'Model') + ': `' + (self.currentAgent.model_name || '?') + '`', meta: '', tools: [] });
              self.scrollToBottom();
            }
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.noAgent', 'No agent selected.'), meta: '', tools: [] });
            self.scrollToBottom();
          }
          break;
        case '/clear':
          self.messages = [];
          break;
        case '/exit':
          LibreFangAPI.wsDisconnect();
          self._wsAgent = null;
          self.currentAgent = null;
          self.messages = [];
          window.dispatchEvent(new Event('close-chat'));
          break;
        case '/budget':
          LibreFangAPI.get('/api/budget').then(function(b) {
            var unlimitedLabel = self.t('agentChat.sys.unlimited', 'unlimited');
            var fmt = function(v) { return v > 0 ? '$' + v.toFixed(2) : unlimitedLabel; };
            self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.budgetStatus', 'Budget Status') + '**\n' +
              '- ' + self.t('agentChat.sys.hourly', 'Hourly') + ': $' + (b.hourly_spend||0).toFixed(4) + ' / ' + fmt(b.hourly_limit) + '\n' +
              '- ' + self.t('agentChat.sys.daily', 'Daily') + ': $' + (b.daily_spend||0).toFixed(4) + ' / ' + fmt(b.daily_limit) + '\n' +
              '- ' + self.t('agentChat.sys.monthly', 'Monthly') + ': $' + (b.monthly_spend||0).toFixed(4) + ' / ' + fmt(b.monthly_limit), meta: '', tools: [] });
            self.scrollToBottom();
          }).catch(function() {});
          break;
        case '/peers':
          LibreFangAPI.get('/api/network/status').then(function(ns) {
            self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.ofpNetwork', 'OFP Network') + '**\n' +
              '- ' + self.t('agentChat.sys.status', 'Status') + ': ' + (ns.enabled ? self.t('agentChat.sys.enabled', 'Enabled') : self.t('agentChat.sys.disabled', 'Disabled')) + '\n' +
              '- ' + self.t('agentChat.sys.connectedPeers', 'Connected peers') + ': ' + (ns.connected_peers||0) + ' / ' + (ns.total_peers||0), meta: '', tools: [] });
            self.scrollToBottom();
          }).catch(function() {});
          break;
        case '/a2a':
          LibreFangAPI.get('/api/a2a/agents').then(function(res) {
            var agents = res.agents || [];
            if (!agents.length) {
              self.messages.push({ id: ++msgId, role: 'system', text: self.t('agentChat.sys.noA2aAgents', 'No external A2A agents discovered.'), meta: '', tools: [] });
            } else {
              var lines = agents.map(function(a) { return '- **' + a.name + '** — ' + a.url; });
              self.messages.push({ id: ++msgId, role: 'system', text: '**' + self.t('agentChat.sys.a2aAgents', 'A2A Agents') + ' (' + agents.length + ')**\n' + lines.join('\n'), meta: '', tools: [] });
            }
            self.scrollToBottom();
          }).catch(function() {});
          break;
      }
    },

    selectAgent(agent) {
      this.currentAgent = agent;
      this.messages = [];
      this.connectWs(agent.id);
      // Show welcome tips on first use
      if (!localStorage.getItem('of-chat-tips-seen')) {
        var localMsgId = 0;
        this.messages.push({
          id: ++localMsgId,
          role: 'system',
          text: this.t('agentChat.welcomeTips', '**Welcome to LibreFang Chat!**'),
          meta: '',
          tools: []
        });
        localStorage.setItem('of-chat-tips-seen', 'true');
      }
      // Focus input after agent selection
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('msg-input');
        if (el) el.focus();
      });
    },

    async loadSession(agentId) {
      var self = this;
      try {
        var data = await LibreFangAPI.get('/api/agents/' + agentId + '/session');
        if (data.messages && data.messages.length) {
          self.messages = data.messages.map(function(m) {
            var role = m.role === 'User' ? 'user' : (m.role === 'System' ? 'system' : 'agent');
            var text = typeof m.content === 'string' ? m.content : JSON.stringify(m.content);
            // Sanitize any raw function-call text from history
            text = self.sanitizeToolText(text);
            // Build tool cards from historical tool data
            var tools = (m.tools || []).map(function(t, idx) {
              return {
                id: (t.name || 'tool') + '-hist-' + idx,
                name: t.name || 'unknown',
                running: false,
                expanded: false,
                input: t.input || '',
                result: t.result || '',
                is_error: !!t.is_error
              };
            });
            var images = (m.images || []).map(function(img) {
              return { file_id: img.file_id, filename: img.filename || 'image' };
            });
            return { id: ++msgId, role: role, text: text, meta: '', tools: tools, images: images };
          });
          self.$nextTick(function() { self.scrollToBottom(); });
        }
      } catch(e) { /* silent */ }
    },

    // Multi-session: load session list for current agent
    async loadSessions(agentId) {
      try {
        var data = await LibreFangAPI.get('/api/agents/' + agentId + '/sessions');
        this.sessions = data.sessions || [];
      } catch(e) { this.sessions = []; }
    },

    // Multi-session: create a new session
    async createSession() {
      if (!this.currentAgent) return;
      var label = prompt(this.t('agentChat.newSessionPrompt', 'Session name (optional):'));
      if (label === null) return; // cancelled
      try {
        await LibreFangAPI.post('/api/agents/' + this.currentAgent.id + '/sessions', {
          label: label.trim() || undefined
        });
        await this.loadSessions(this.currentAgent.id);
        await this.loadSession(this.currentAgent.id);
        this.messages = [];
        this.scrollToBottom();
        if (typeof LibreFangToast !== 'undefined') LibreFangToast.success(this.t('agentChat.newSessionCreated', 'New session created'));
      } catch(e) {
        if (typeof LibreFangToast !== 'undefined') LibreFangToast.error(this.t('agentChat.newSessionFailed', 'Failed to create session'));
      }
    },

    // Multi-session: switch to an existing session
    async switchSession(sessionId) {
      if (!this.currentAgent) return;
      try {
        await LibreFangAPI.post('/api/agents/' + this.currentAgent.id + '/sessions/' + sessionId + '/switch', {});
        this.messages = [];
        await this.loadSession(this.currentAgent.id);
        await this.loadSessions(this.currentAgent.id);
        // Reconnect WebSocket for new session
        this._wsAgent = null;
        this.connectWs(this.currentAgent.id);
      } catch(e) {
        if (typeof LibreFangToast !== 'undefined') LibreFangToast.error(this.t('agentChat.switchSessionFailed', 'Failed to switch session'));
      }
    },

    connectWs(agentId) {
      if (this._wsAgent === agentId) return;
      this._wsAgent = agentId;
      var self = this;

      LibreFangAPI.wsConnect(agentId, {
        onOpen: function() {
          Alpine.store('app').wsConnected = true;
        },
        onMessage: function(data) { self.handleWsMessage(data); },
        onClose: function() {
          Alpine.store('app').wsConnected = false;
          self._wsAgent = null;
        },
        onError: function() {
          Alpine.store('app').wsConnected = false;
          self._wsAgent = null;
        }
      });
    },

    handleWsMessage(data) {
      switch (data.type) {
        case 'connected': break;

        // Legacy thinking event (backward compat)
        case 'thinking':
          if (!this.messages.length || !this.messages[this.messages.length - 1].thinking) {
            var thinkLabel = data.level
              ? this.t('agentChat.thinkingWithLevel', 'Thinking ({level})...', { level: data.level })
              : this.t('agentChat.processing', 'Processing...');
            this.messages.push({ id: ++msgId, role: 'agent', text: thinkLabel, meta: '', thinking: true, streaming: true, tools: [] });
            this.scrollToBottom();
            this._resetTypingTimeout();
          } else if (data.level) {
            var lastThink = this.messages[this.messages.length - 1];
            if (lastThink && lastThink.thinking) {
              lastThink.text = this.t('agentChat.thinkingWithLevel', 'Thinking ({level})...', { level: data.level });
            }
          }
          break;

        // New typing lifecycle
        case 'typing':
          if (data.state === 'start') {
            if (!this.messages.length || !this.messages[this.messages.length - 1].thinking) {
              this.messages.push({ id: ++msgId, role: 'agent', text: this.t('agentChat.processing', 'Processing...'), meta: '', thinking: true, streaming: true, tools: [] });
              this.scrollToBottom();
            }
            this._resetTypingTimeout();
          } else if (data.state === 'tool') {
            var typingMsg = this.messages.length ? this.messages[this.messages.length - 1] : null;
            if (typingMsg && (typingMsg.thinking || typingMsg.streaming)) {
              typingMsg.text = this.t('agentChat.usingTool', 'Using {tool}...', {
                tool: data.tool || 'tool'
              });
            }
            this._resetTypingTimeout();
          } else if (data.state === 'stop') {
            this._clearTypingTimeout();
          }
          break;

        case 'phase':
          // Show tool/phase progress so the user sees the agent is working
          var phaseMsg = this.messages.length ? this.messages[this.messages.length - 1] : null;
          if (phaseMsg && (phaseMsg.thinking || phaseMsg.streaming)) {
            // Skip phases that have no user-meaningful display text — "streaming"
            // and "done" are lifecycle signals, not status to show in the chat bubble.
            if (data.phase === 'streaming' || data.phase === 'done') {
              break;
            }
            // Context warning: show prominently as a separate system message
            if (data.phase === 'context_warning') {
              var cwDetail = data.detail || 'Context limit reached.';
              this.messages.push({ id: ++msgId, role: 'system', text: cwDetail, meta: '', tools: [] });
            } else if (data.phase === 'thinking' && this.thinkingMode === 'stream') {
              // Stream reasoning tokens to a collapsible panel
              if (!phaseMsg._reasoning) phaseMsg._reasoning = '';
              phaseMsg._reasoning += (data.detail || '') + '\n';
              phaseMsg.text = '<details><summary>Reasoning...</summary>\n\n' + phaseMsg._reasoning + '</details>';
            } else if (phaseMsg.thinking) {
              // Only update text on messages still in thinking state (not yet
              // receiving streamed content) to avoid overwriting accumulated text.
              var phaseDetail;
              if (data.phase === 'tool_use') {
                phaseDetail = this.t('agentChat.usingTool', 'Using {tool}...', {
                  tool: data.detail || 'tool'
                });
              } else if (data.phase === 'thinking') {
                phaseDetail = this.t('agentChat.thinking', 'Thinking...');
              } else {
                phaseDetail = data.detail || this.t('agentChat.working', 'Working...');
              }
              phaseMsg.text = phaseDetail;
            }
          }
          this.scrollToBottom();
          break;

        case 'text_delta':
          var last = this.messages.length ? this.messages[this.messages.length - 1] : null;
          if (last && last.streaming) {
            if (last.thinking) { last.text = ''; last.thinking = false; }
            // If we already detected a text-based tool call, skip further text
            if (last._toolTextDetected) break;
            last.text += data.content;
            // Detect function-call patterns streamed as text and convert to tool cards
            var fcIdx = last.text.search(/\w+<\/function[=,>]/);
            if (fcIdx === -1) fcIdx = last.text.search(/<function=\w+>/);
            if (fcIdx !== -1) {
              var fcPart = last.text.substring(fcIdx);
              var toolMatch = fcPart.match(/^(\w+)<\/function/) || fcPart.match(/^<function=(\w+)>/);
              last.text = last.text.substring(0, fcIdx).trim();
              last._toolTextDetected = true;
              if (toolMatch) {
                if (!last.tools) last.tools = [];
                var inputMatch = fcPart.match(/[=,>]\s*(\{[\s\S]*)/);
                last.tools.push({
                  id: toolMatch[1] + '-txt-' + Date.now(),
                  name: toolMatch[1],
                  running: true,
                  expanded: false,
                  input: inputMatch ? inputMatch[1].replace(/<\/function>?\s*$/, '').trim() : '',
                  result: '',
                  is_error: false
                });
              }
            }
            this.tokenCount = Math.round(last.text.length / 4);
          } else {
            this.messages.push({ id: ++msgId, role: 'agent', text: data.content, meta: '', streaming: true, tools: [] });
          }
          this.scrollToBottom();
          break;

        case 'tool_start':
          var lastMsg = this.messages.length ? this.messages[this.messages.length - 1] : null;
          if (lastMsg && lastMsg.streaming) {
            if (!lastMsg.tools) lastMsg.tools = [];
            lastMsg.tools.push({ id: data.tool + '-' + Date.now(), name: data.tool, running: true, expanded: false, input: '', result: '', is_error: false });
          }
          this.scrollToBottom();
          break;

        case 'tool_end':
          // Tool call parsed by LLM — update tool card with input params
          var lastMsg2 = this.messages.length ? this.messages[this.messages.length - 1] : null;
          if (lastMsg2 && lastMsg2.tools) {
            for (var ti = lastMsg2.tools.length - 1; ti >= 0; ti--) {
              if (lastMsg2.tools[ti].name === data.tool && lastMsg2.tools[ti].running) {
                lastMsg2.tools[ti].input = data.input || '';
                break;
              }
            }
          }
          break;

        case 'tool_result':
          // Tool execution completed — update tool card with result
          var lastMsg3 = this.messages.length ? this.messages[this.messages.length - 1] : null;
          if (lastMsg3 && lastMsg3.tools) {
            for (var ri = lastMsg3.tools.length - 1; ri >= 0; ri--) {
              if (lastMsg3.tools[ri].name === data.tool && lastMsg3.tools[ri].running) {
                lastMsg3.tools[ri].running = false;
                lastMsg3.tools[ri].result = data.result || '';
                lastMsg3.tools[ri].is_error = !!data.is_error;
                // Extract image URLs from image_generate or browser_screenshot results
                if ((data.tool === 'image_generate' || data.tool === 'browser_screenshot') && !data.is_error) {
                  try {
                    var parsed = JSON.parse(data.result);
                    if (parsed.image_urls && parsed.image_urls.length) {
                      lastMsg3.tools[ri]._imageUrls = parsed.image_urls;
                    }
                  } catch(e) { /* not JSON */ }
                }
                // Extract audio file path from text_to_speech results
                if (data.tool === 'text_to_speech' && !data.is_error) {
                  try {
                    var ttsResult = JSON.parse(data.result);
                    if (ttsResult.saved_to) {
                      lastMsg3.tools[ri]._audioFile = ttsResult.saved_to;
                      lastMsg3.tools[ri]._audioDuration = ttsResult.duration_estimate_ms;
                    }
                  } catch(e) { /* not JSON */ }
                }
                break;
              }
            }
          }
          this.scrollToBottom();
          break;

        case 'response':
          this._clearTypingTimeout();
          // Update context pressure from response
          if (data.context_pressure) {
            this.contextPressure = data.context_pressure;
          }
          // Collect streamed text before removing streaming messages
          var streamedText = '';
          var streamedTools = [];
          this.messages.forEach(function(m) {
            if (m.streaming && !m.thinking && m.role === 'agent') {
              streamedText += m.text || '';
              streamedTools = streamedTools.concat(m.tools || []);
            }
          });
          streamedTools.forEach(function(t) {
            t.running = false;
            // Text-detected tool calls (model leaked as text) — mark as not executed
            if (t.id && t.id.indexOf('-txt-') !== -1 && !t.result) {
              t.result = 'Model attempted this call as text (not executed via tool system)';
              t.is_error = true;
            }
          });
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          var meta = (data.input_tokens || 0) + ' in / ' + (data.output_tokens || 0) + ' out';
          if (data.cost_usd != null) meta += ' | $' + data.cost_usd.toFixed(4);
          if (data.iterations) meta += ' | ' + data.iterations + ' iter';
          if (data.fallback_model) meta += ' | fallback: ' + data.fallback_model;
          // Use server response if non-empty, otherwise preserve accumulated streamed text
          var finalText = (data.content && data.content.trim()) ? data.content : streamedText;
          // Strip raw function-call JSON that some models leak as text
          finalText = this.sanitizeToolText(finalText);
          // If text is empty but tools ran, show a summary
          if (!finalText.trim() && streamedTools.length) {
            finalText = '';
          }
          this.messages.push({ id: ++msgId, role: 'agent', text: finalText, meta: meta, tools: streamedTools, ts: Date.now() });
          this.sending = false;
          this.tokenCount = 0;
          this.scrollToBottom();
          var self3 = this;
          this.$nextTick(function() {
            var el = document.getElementById('msg-input'); if (el) el.focus();
            self3._processQueue();
          });
          break;

        case 'silent_complete':
          // Agent intentionally chose not to reply (NO_REPLY)
          this._clearTypingTimeout();
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          this.sending = false;
          this.tokenCount = 0;
          // No message bubble added — the agent was silent
          var selfSilent = this;
          this.$nextTick(function() { selfSilent._processQueue(); });
          break;

        case 'error':
          this._clearTypingTimeout();
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          this.messages.push({
            id: ++msgId,
            role: 'system',
            text: this.t('agentChat.errorPrefix', 'Error: {message}', { message: data.content }),
            meta: '',
            tools: [],
            ts: Date.now()
          });
          this.sending = false;
          this.tokenCount = 0;
          this.scrollToBottom();
          var self2 = this;
          this.$nextTick(function() {
            var el = document.getElementById('msg-input'); if (el) el.focus();
            self2._processQueue();
          });
          break;

        case 'agents_updated':
          if (data.agents) {
            Alpine.store('app').agents = data.agents;
            Alpine.store('app').agentCount = data.agents.length;
          }
          break;

        case 'command_result':
          // Update context pressure if included in command result
          if (data.context_pressure) {
            this.contextPressure = data.context_pressure;
          }
          this.messages.push({ id: ++msgId, role: 'system', text: data.message || 'Command executed.', meta: '', tools: [] });
          this.scrollToBottom();
          break;

        case 'canvas':
          // Agent presented an interactive canvas — render it in an iframe sandbox
          var canvasHtml = '<div class="canvas-panel" style="border:1px solid var(--border);border-radius:8px;margin:8px 0;overflow:hidden;">';
          canvasHtml += '<div style="padding:6px 12px;background:var(--surface);border-bottom:1px solid var(--border);font-size:0.85em;display:flex;justify-content:space-between;align-items:center;">';
          canvasHtml += '<span>' + (data.title || 'Canvas') + '</span>';
          canvasHtml += '<span style="opacity:0.5;font-size:0.8em;">' + (data.canvas_id || '').substring(0, 8) + '</span></div>';
          canvasHtml += '<iframe sandbox="allow-scripts" srcdoc="' + (data.html || '').replace(/"/g, '&quot;') + '" ';
          canvasHtml += 'style="width:100%;min-height:300px;border:none;background:#fff;" loading="lazy"></iframe></div>';
          this.messages.push({ id: ++msgId, role: 'agent', text: canvasHtml, meta: 'canvas', isHtml: true, tools: [] });
          this.scrollToBottom();
          break;

        case 'pong': break;
      }
    },

    // Format timestamp for display
    formatTime: function(ts) {
      if (!ts) return '';
      var d = new Date(ts);
      var h = d.getHours();
      var m = d.getMinutes();
      var ampm = h >= 12 ? 'PM' : 'AM';
      h = h % 12 || 12;
      return h + ':' + (m < 10 ? '0' : '') + m + ' ' + ampm;
    },

    // Copy message text to clipboard
    copyMessage: function(msg) {
      var text = msg.text || '';
      navigator.clipboard.writeText(text).then(function() {
        msg._copied = true;
        setTimeout(function() { msg._copied = false; }, 2000);
      }).catch(function() {});
    },

    // Process queued messages after current response completes
    _processQueue: function() {
      if (!this.messageQueue.length || this.sending) return;
      var next = this.messageQueue.shift();
      this._sendPayload(next.text, next.files, next.images);
    },

    async sendMessage() {
      if (!this.currentAgent || (!this.inputText.trim() && !this.attachments.length)) return;
      var text = this.inputText.trim();

      // Handle slash commands
      if (text.startsWith('/') && !this.attachments.length) {
        var cmd = text.split(' ')[0].toLowerCase();
        var cmdArgs = text.substring(cmd.length).trim();
        var matched = this.slashCommands.find(function(c) { return c.cmd === cmd; });
        if (matched) {
          this.executeSlashCommand(matched.cmd, cmdArgs);
          return;
        }
      }

      this.inputText = '';

      // Reset textarea height to single line
      var ta = document.getElementById('msg-input');
      if (ta) ta.style.height = '';

      // Upload attachments first if any
      var fileRefs = [];
      var uploadedFiles = [];
      if (this.attachments.length) {
        for (var i = 0; i < this.attachments.length; i++) {
          var att = this.attachments[i];
          att.uploading = true;
          try {
            var uploadRes = await LibreFangAPI.upload(this.currentAgent.id, att.file);
            fileRefs.push('[File: ' + att.file.name + ']');
            uploadedFiles.push({ file_id: uploadRes.file_id, filename: uploadRes.filename, content_type: uploadRes.content_type });
          } catch(e) {
            LibreFangToast.error(this.t('agentChat.failedUploadFile', 'Failed to upload {name}', {
              name: att.file.name
            }));
            fileRefs.push('[File: ' + att.file.name + ' (upload failed)]');
          }
          att.uploading = false;
        }
        // Clean up previews
        for (var j = 0; j < this.attachments.length; j++) {
          if (this.attachments[j].preview) URL.revokeObjectURL(this.attachments[j].preview);
        }
        this.attachments = [];
      }

      // Build final message text
      var finalText = text;
      if (fileRefs.length) {
        finalText = (text ? text + '\n' : '') + fileRefs.join('\n');
      }

      // Collect image references for inline rendering
      var msgImages = uploadedFiles.filter(function(f) { return f.content_type && f.content_type.startsWith('image/'); });

      // Always show user message immediately
      this.messages.push({ id: ++msgId, role: 'user', text: finalText, meta: '', tools: [], images: msgImages, ts: Date.now() });
      this.scrollToBottom();
      localStorage.setItem('of-first-msg', 'true');

      // If already streaming, queue this message
      if (this.sending) {
        this.messageQueue.push({ text: finalText, files: uploadedFiles, images: msgImages });
        return;
      }

      this._sendPayload(finalText, uploadedFiles, msgImages);
    },

    async _sendPayload(finalText, uploadedFiles, msgImages) {
      this.sending = true;

      // Try WebSocket first
      var wsPayload = { type: 'message', content: finalText };
      if (uploadedFiles && uploadedFiles.length) wsPayload.attachments = uploadedFiles;
      if (LibreFangAPI.wsSend(wsPayload)) {
        this.messages.push({ id: ++msgId, role: 'agent', text: '', meta: '', thinking: true, streaming: true, tools: [], ts: Date.now() });
        this.scrollToBottom();
        return;
      }

      // HTTP fallback
      if (!LibreFangAPI.isWsConnected()) {
        LibreFangToast.info(this.t('agentChat.usingHttpMode', 'Using HTTP mode (no streaming)'));
      }
      this.messages.push({ id: ++msgId, role: 'agent', text: '', meta: '', thinking: true, tools: [], ts: Date.now() });
      this.scrollToBottom();

      try {
        var httpBody = { message: finalText };
        if (uploadedFiles && uploadedFiles.length) httpBody.attachments = uploadedFiles;
        var res = await LibreFangAPI.post('/api/agents/' + this.currentAgent.id + '/message', httpBody);
        this.messages = this.messages.filter(function(m) { return !m.thinking; });
        var httpMeta = (res.input_tokens || 0) + ' in / ' + (res.output_tokens || 0) + ' out';
        if (res.cost_usd != null) httpMeta += ' | $' + res.cost_usd.toFixed(4);
        if (res.iterations) httpMeta += ' | ' + res.iterations + ' iter';
        this.messages.push({ id: ++msgId, role: 'agent', text: res.response, meta: httpMeta, tools: [], ts: Date.now() });
      } catch(e) {
        this.messages = this.messages.filter(function(m) { return !m.thinking; });
        this.messages.push({
          id: ++msgId,
          role: 'system',
          text: this.t('agentChat.errorPrefix', 'Error: {message}', { message: e.message }),
          meta: '',
          tools: [],
          ts: Date.now()
        });
      }
      this.sending = false;
      this.scrollToBottom();
      // Process next queued message
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('msg-input'); if (el) el.focus();
        self._processQueue();
      });
    },

    // Stop the current agent run
    stopAgent: function() {
      if (!this.currentAgent) return;
      var self = this;
      LibreFangAPI.post('/api/agents/' + this.currentAgent.id + '/stop', {}).then(function(res) {
        self.messages.push({
          id: ++msgId,
          role: 'system',
          text: res.message || self.t('agentChat.runCancelled', 'Run cancelled'),
          meta: '',
          tools: [],
          ts: Date.now()
        });
        self.sending = false;
        self.scrollToBottom();
        self.$nextTick(function() { self._processQueue(); });
      }).catch(function(e) {
        LibreFangToast.error(self.t('agentChat.stopFailed', 'Stop failed: {message}', {
          message: e.message
        }));
      });
    },

    killAgent() {
      if (!this.currentAgent) return;
      var self = this;
      var name = this.currentAgent.name;
      LibreFangToast.confirm(
        this.t('agentChat.stopAgentTitle', 'Stop Agent'),
        this.t('agentChat.stopAgentConfirm', 'Stop agent "{name}"? The agent will be shut down.', { name: name }),
        async function() {
        try {
          await LibreFangAPI.del('/api/agents/' + self.currentAgent.id);
          LibreFangAPI.wsDisconnect();
          self._wsAgent = null;
          self.currentAgent = null;
          self.messages = [];
          LibreFangToast.success(self.t('agentChat.agentStopped', 'Agent "{name}" stopped', { name: name }));
          Alpine.store('app').refreshAgents();
        } catch(e) {
          LibreFangToast.error(self.t('agentChat.stopFailed', 'Stop failed: {message}', {
            message: e.message
          }));
        }
      });
    },

    scrollToBottom() {
      var self = this;
      var el = document.getElementById('messages');
      if (el) self.$nextTick(function() { el.scrollTop = el.scrollHeight; });
    },

    addFiles(files) {
      var self = this;
      var allowed = ['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'text/plain', 'application/pdf',
                      'text/markdown', 'application/json', 'text/csv'];
      var allowedExts = ['.txt', '.pdf', '.md', '.json', '.csv'];
      for (var i = 0; i < files.length; i++) {
        var file = files[i];
        if (file.size > 10 * 1024 * 1024) {
          LibreFangToast.warn(this.t('agentChat.fileTooLarge', 'File "{name}" exceeds 10MB limit', {
            name: file.name
          }));
          continue;
        }
        var typeOk = allowed.indexOf(file.type) !== -1;
        if (!typeOk) {
          var ext = file.name.lastIndexOf('.') !== -1 ? file.name.substring(file.name.lastIndexOf('.')).toLowerCase() : '';
          typeOk = allowedExts.indexOf(ext) !== -1 || file.type.startsWith('image/');
        }
        if (!typeOk) {
          LibreFangToast.warn(this.t('agentChat.fileTypeNotSupported', 'File type not supported: {name}', {
            name: file.name
          }));
          continue;
        }
        var preview = null;
        if (file.type.startsWith('image/')) {
          preview = URL.createObjectURL(file);
        }
        self.attachments.push({ file: file, preview: preview, uploading: false });
      }
    },

    removeAttachment(idx) {
      var att = this.attachments[idx];
      if (att && att.preview) URL.revokeObjectURL(att.preview);
      this.attachments.splice(idx, 1);
    },

    handleDrop(e) {
      e.preventDefault();
      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length) {
        this.addFiles(e.dataTransfer.files);
      }
    },

    isGrouped(idx) {
      if (idx === 0) return false;
      var prev = this.messages[idx - 1];
      var curr = this.messages[idx];
      return prev && curr && prev.role === curr.role && !curr.thinking && !prev.thinking;
    },

    // Strip raw function-call text that some models (Llama, Groq, etc.) leak into output.
    // These models don't use proper tool_use blocks — they output function calls as plain text.
    sanitizeToolText: function(text) {
      if (!text) return text;
      // Pattern: tool_name</function={"key":"value"} or tool_name</function,{...}
      text = text.replace(/\s*\w+<\/function[=,]?\s*\{[\s\S]*$/gm, '');
      // Pattern: <function=tool_name>{...}</function>
      text = text.replace(/<function=\w+>[\s\S]*?<\/function>/g, '');
      // Pattern: tool_name{"type":"function",...}
      text = text.replace(/\s*\w+\{"type"\s*:\s*"function"[\s\S]*$/gm, '');
      // Pattern: lone </function...> tags
      text = text.replace(/<\/function[^>]*>/g, '');
      // Pattern: <|python_tag|> or similar special tokens
      text = text.replace(/<\|[\w_]+\|>/g, '');
      return text.trim();
    },

    formatToolJson: function(text) {
      if (!text) return '';
      try { return JSON.stringify(JSON.parse(text), null, 2); }
      catch(e) { return text; }
    },

    // Voice: start recording
    startRecording: async function() {
      if (this.recording) return;
      try {
        var stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        var mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus') ? 'audio/webm;codecs=opus' :
                       MediaRecorder.isTypeSupported('audio/webm') ? 'audio/webm' : 'audio/ogg';
        this._audioChunks = [];
        this._mediaRecorder = new MediaRecorder(stream, { mimeType: mimeType });
        var self = this;
        this._mediaRecorder.ondataavailable = function(e) {
          if (e.data.size > 0) self._audioChunks.push(e.data);
        };
        this._mediaRecorder.onstop = function() {
          stream.getTracks().forEach(function(t) { t.stop(); });
          self._handleRecordingComplete();
        };
        this._mediaRecorder.start(250);
        this.recording = true;
        this.recordingTime = 0;
        this._recordingTimer = setInterval(function() { self.recordingTime++; }, 1000);
      } catch(e) {
        if (typeof LibreFangToast !== 'undefined') {
          LibreFangToast.error(this.t('agentChat.microphoneDenied', 'Microphone access denied'));
        }
      }
    },

    // Voice: stop recording
    stopRecording: function() {
      if (!this.recording || !this._mediaRecorder) return;
      this._mediaRecorder.stop();
      this.recording = false;
      if (this._recordingTimer) { clearInterval(this._recordingTimer); this._recordingTimer = null; }
    },

    // Voice: handle completed recording — upload and transcribe
    _handleRecordingComplete: async function() {
      if (!this._audioChunks.length || !this.currentAgent) return;
      var blob = new Blob(this._audioChunks, { type: this._audioChunks[0].type || 'audio/webm' });
      this._audioChunks = [];
      if (blob.size < 100) return; // too small

      // Show a temporary "Transcribing..." message
      this.messages.push({
        id: ++msgId,
        role: 'system',
        text: this.t('agentChat.transcribingAudio', 'Transcribing audio...'),
        thinking: true,
        ts: Date.now(),
        tools: []
      });
      this.scrollToBottom();

      try {
        // Upload audio file
        var ext = blob.type.includes('webm') ? 'webm' : blob.type.includes('ogg') ? 'ogg' : 'mp3';
        var file = new File([blob], 'voice_' + Date.now() + '.' + ext, { type: blob.type });
        var upload = await LibreFangAPI.upload(this.currentAgent.id, file);

        // Remove the "Transcribing..." message
        this.messages = this.messages.filter(function(m) { return !m.thinking || m.role !== 'system'; });

        // Use server-side transcription if available, otherwise fall back to placeholder
        var text = (upload.transcription && upload.transcription.trim())
          ? upload.transcription.trim()
          : '[Voice message - audio: ' + upload.filename + ']';
        this._sendPayload(text, [upload], []);
      } catch(e) {
        this.messages = this.messages.filter(function(m) { return !m.thinking || m.role !== 'system'; });
        if (typeof LibreFangToast !== 'undefined') {
          LibreFangToast.error(this.t('agentChat.failedUploadAudio', 'Failed to upload audio: {message}', {
            message: e.message || 'unknown error'
          }));
        }
      }
    },

    // Voice: format recording time as MM:SS
    formatRecordingTime: function() {
      var m = Math.floor(this.recordingTime / 60);
      var s = this.recordingTime % 60;
      return (m < 10 ? '0' : '') + m + ':' + (s < 10 ? '0' : '') + s;
    },

    // Search: toggle open/close
    toggleSearch: function() {
      this.searchOpen = !this.searchOpen;
      if (this.searchOpen) {
        var self = this;
        this.$nextTick(function() {
          var el = document.getElementById('chat-search-input');
          if (el) el.focus();
        });
      } else {
        this.searchQuery = '';
      }
    },

    // Search: filter messages by query
    get filteredMessages() {
      if (!this.searchQuery.trim()) return this.messages;
      var q = this.searchQuery.toLowerCase();
      return this.messages.filter(function(m) {
        return (m.text && m.text.toLowerCase().indexOf(q) !== -1) ||
               (m.tools && m.tools.some(function(t) { return t.name.toLowerCase().indexOf(q) !== -1; }));
      });
    },

    // Search: highlight matched text in a string
    highlightSearch: function(html) {
      if (!this.searchQuery.trim() || !html) return html;
      var q = this.searchQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      var regex = new RegExp('(' + q + ')', 'gi');
      return html.replace(regex, '<mark style="background:var(--warning);color:var(--bg);border-radius:2px;padding:0 2px">$1</mark>');
    },

    renderMarkdown: renderMarkdown,
    escapeHtml: escapeHtml
  };
}
