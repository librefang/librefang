// LibreFang Workflows Page — Workflow builder + run history
'use strict';

function workflowsPage() {
  return {
    _currentLang: typeof i18n !== 'undefined' ? i18n.getLanguage() : 'en',
    // -- Workflows state --
    workflows: [],
    showCreateModal: false,
    runModal: null,
    runInput: '',
    runResult: '',
    running: false,
    loading: true,
    loadError: '',
    newWf: { name: '', description: '', steps: [{ name: '', agent_name: '', mode: 'sequential', prompt: '{{input}}' }] },

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

    stepCountText(workflow) {
      var count = Array.isArray(workflow.steps) ? workflow.steps.length : workflow.steps;
      return this.t('workflowsPage.stepsCount', '{count} step(s)', { count: count || 0 });
    },

    runModalTitle() {
      if (!this.runModal) return '';
      return this.t('workflowsPage.runTitle', 'Run: {name}', { name: this.runModal.name });
    },

    // -- Workflows methods --
    async loadWorkflows() {
      this.loading = true;
      this.loadError = '';
      try {
        this.workflows = await LibreFangAPI.get('/api/workflows');
      } catch(e) {
        this.workflows = [];
        this.loadError = e.message || this.t('workflowsPage.loadError', 'Could not load workflows.');
      }
      this.loading = false;
    },

    async loadData() { return this.loadWorkflows(); },

    async createWorkflow() {
      var steps = this.newWf.steps.map(function(s) {
        return { name: s.name || 'step', agent_name: s.agent_name, mode: s.mode, prompt: s.prompt || '{{input}}' };
      });
      try {
        var wfName = this.newWf.name;
        await LibreFangAPI.post('/api/workflows', { name: wfName, description: this.newWf.description, steps: steps });
        this.showCreateModal = false;
        this.newWf = { name: '', description: '', steps: [{ name: '', agent_name: '', mode: 'sequential', prompt: '{{input}}' }] };
        LibreFangToast.success(this.t('workflowsPage.created', 'Workflow "{name}" created', { name: wfName }));
        await this.loadWorkflows();
      } catch(e) {
        LibreFangToast.error(this.t('workflowsPage.createFailed', 'Failed to create workflow: {message}', {
          message: e.message || this.t('status.unknown', 'unknown')
        }));
      }
    },

    showRunModal(wf) {
      this.runModal = wf;
      this.runInput = '';
      this.runResult = '';
    },

    async executeWorkflow() {
      if (!this.runModal) return;
      this.running = true;
      this.runResult = '';
      try {
        var res = await LibreFangAPI.post('/api/workflows/' + this.runModal.id + '/run', { input: this.runInput });
        this.runResult = res.output || JSON.stringify(res, null, 2);
        LibreFangToast.success(this.t('workflowsPage.completed', 'Workflow completed'));
      } catch(e) {
        this.runResult = this.t('workflowsPage.errorMessage', 'Error: {message}', {
          message: e.message || this.t('status.unknown', 'unknown')
        });
        LibreFangToast.error(this.t('workflowsPage.runFailed', 'Workflow failed: {message}', {
          message: e.message || this.t('status.unknown', 'unknown')
        }));
      }
      this.running = false;
    },

    async viewRuns(wf) {
      try {
        var runs = await LibreFangAPI.get('/api/workflows/' + wf.id + '/runs');
        this.runResult = JSON.stringify(runs, null, 2);
        this.runModal = wf;
      } catch(e) {
        LibreFangToast.error(this.t('workflowsPage.historyFailed', 'Failed to load run history: {message}', {
          message: e.message || this.t('status.unknown', 'unknown')
        }));
      }
    }
  };
}
