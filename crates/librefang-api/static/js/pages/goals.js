// LibreFang Goals Page — Hierarchical goal management with tree view
'use strict';

function goalsPage() {
  return {
    goals: [],
    loading: true,
    loadError: '',

    // Create form
    showCreateForm: false,
    newGoal: { title: '', description: '', parent_id: '', agent_id: '', status: 'pending' },
    creating: false,

    // Edit form
    editingGoal: null,
    editForm: { title: '', description: '', status: 'pending', progress: 0, parent_id: '', agent_id: '' },
    saving: false,

    // Tree state — tracks expanded nodes
    expandedIds: {},

    init() {
      this.loadGoals();
    },

    async loadGoals() {
      this.loading = true;
      this.loadError = '';
      try {
        var r = await fetch('/api/goals');
        var data = await r.json();
        this.goals = data.goals || [];
      } catch (e) {
        this.loadError = e.message || 'Failed to load goals';
      }
      this.loading = false;
    },

    // Build tree structure from flat list
    get rootGoals() {
      return this.goals.filter(function(g) { return !g.parent_id; });
    },

    childrenOf(parentId) {
      return this.goals.filter(function(g) { return g.parent_id === parentId; });
    },

    hasChildren(goalId) {
      return this.goals.some(function(g) { return g.parent_id === goalId; });
    },

    toggleExpand(goalId) {
      this.expandedIds[goalId] = !this.expandedIds[goalId];
    },

    isExpanded(goalId) {
      return !!this.expandedIds[goalId];
    },

    // Depth calculation for indentation
    depthOf(goal) {
      var depth = 0;
      var current = goal;
      var seen = {};
      while (current && current.parent_id && !seen[current.id]) {
        seen[current.id] = true;
        depth++;
        var pid = current.parent_id;
        current = this.goals.find(function(g) { return g.id === pid; });
      }
      return depth;
    },

    // Flatten the tree for rendering (DFS order)
    get flatTree() {
      var self = this;
      var result = [];
      var visited = {};
      function walk(parentId, depth) {
        var children = parentId
          ? self.goals.filter(function(g) { return g.parent_id === parentId; })
          : self.goals.filter(function(g) { return !g.parent_id; });
        children.forEach(function(g) {
          if (visited[g.id]) return; // guard against circular references
          visited[g.id] = true;
          result.push({ goal: g, depth: depth });
          if (self.isExpanded(g.id)) {
            walk(g.id, depth + 1);
          }
        });
      }
      walk(null, 0);
      return result;
    },

    statusBadgeClass(status) {
      switch (status) {
        case 'completed': return 'badge-success';
        case 'in_progress': return 'badge-warning';
        case 'cancelled': return 'badge-dim';
        default: return 'badge-info';
      }
    },

    statusLabel(status) {
      switch (status) {
        case 'pending': return 'Pending';
        case 'in_progress': return 'In Progress';
        case 'completed': return 'Completed';
        case 'cancelled': return 'Cancelled';
        default: return status;
      }
    },

    // Possible parent goals (all goals except the goal being edited and its descendants)
    possibleParents(excludeId) {
      if (!excludeId) return this.goals;
      var self = this;
      // Collect all descendant IDs to exclude
      var excluded = {};
      excluded[excludeId] = true;
      var changed = true;
      while (changed) {
        changed = false;
        self.goals.forEach(function(g) {
          if (g.parent_id && excluded[g.parent_id] && !excluded[g.id]) {
            excluded[g.id] = true;
            changed = true;
          }
        });
      }
      return this.goals.filter(function(g) { return !excluded[g.id]; });
    },

    async createGoal() {
      if (!this.newGoal.title.trim()) return;
      this.creating = true;
      try {
        var body = {
          title: this.newGoal.title.trim(),
          description: this.newGoal.description.trim(),
          status: this.newGoal.status || 'pending'
        };
        if (this.newGoal.parent_id) body.parent_id = this.newGoal.parent_id;
        if (this.newGoal.agent_id) body.agent_id = this.newGoal.agent_id;

        var r = await fetch('/api/goals', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        });
        if (!r.ok) {
          var err = await r.json();
          alert(err.error || 'Failed to create goal');
        } else {
          this.showCreateForm = false;
          this.newGoal = { title: '', description: '', parent_id: '', agent_id: '', status: 'pending' };
          await this.loadGoals();
        }
      } catch (e) {
        alert(e.message || 'Failed to create goal');
      }
      this.creating = false;
    },

    startEdit(goal) {
      this.editingGoal = goal.id;
      this.editForm = {
        title: goal.title,
        description: goal.description || '',
        status: goal.status,
        progress: goal.progress || 0,
        parent_id: goal.parent_id || '',
        agent_id: goal.agent_id || ''
      };
    },

    cancelEdit() {
      this.editingGoal = null;
    },

    async saveEdit(goalId) {
      this.saving = true;
      try {
        var body = {
          title: this.editForm.title.trim(),
          description: this.editForm.description.trim(),
          status: this.editForm.status,
          progress: parseInt(this.editForm.progress) || 0
        };
        if (this.editForm.parent_id) {
          body.parent_id = this.editForm.parent_id;
        } else {
          body.parent_id = null;
        }
        if (this.editForm.agent_id) {
          body.agent_id = this.editForm.agent_id;
        } else {
          body.agent_id = null;
        }
        var r = await fetch('/api/goals/' + goalId, {
          method: 'PUT',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body)
        });
        if (!r.ok) {
          var err = await r.json();
          alert(err.error || 'Failed to update goal');
        } else {
          this.editingGoal = null;
          await this.loadGoals();
        }
      } catch (e) {
        alert(e.message || 'Failed to update goal');
      }
      this.saving = false;
    },

    async deleteGoal(goalId) {
      var hasKids = this.hasChildren(goalId);
      var msg = hasKids
        ? 'Delete this goal and all its sub-goals?'
        : 'Delete this goal?';
      if (!confirm(msg)) return;
      try {
        var r = await fetch('/api/goals/' + goalId, { method: 'DELETE' });
        if (!r.ok) {
          var err = await r.json();
          alert(err.error || 'Failed to delete goal');
        } else {
          await this.loadGoals();
        }
      } catch (e) {
        alert(e.message || 'Failed to delete goal');
      }
    },

    progressColor(progress) {
      if (progress >= 80) return 'var(--green)';
      if (progress >= 40) return 'var(--accent)';
      return 'var(--text-dim)';
    },

    get stats() {
      var total = this.goals.length;
      var completed = this.goals.filter(function(g) { return g.status === 'completed'; }).length;
      var inProgress = this.goals.filter(function(g) { return g.status === 'in_progress'; }).length;
      var pending = this.goals.filter(function(g) { return g.status === 'pending'; }).length;
      return { total: total, completed: completed, inProgress: inProgress, pending: pending };
    }
  };
}
