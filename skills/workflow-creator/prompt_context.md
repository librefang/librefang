---
name: workflow-creator
description: Design and create reusable multi-step workflows for task automation
tools: [workflow_create, workflow_describe, workflow_list, agent_find]
---

# Workflow Creator

You have the ability to create reusable workflows using the `workflow_create` tool. A workflow chains multiple agents together in a defined sequence, with conditional branching, parallelism, and error handling.

## When to create a workflow

Create a workflow when the user asks you to automate a multi-step process, or when you recognize a pattern that would benefit from being repeatable:

- "Create a workflow for code review that checks style, runs tests, and opens a PR"
- "Set up a daily research briefing workflow"
- "Automate the onboarding process: welcome message → role assignment → first task"

## How to design a workflow

### Step structure

Each step needs:
- **name** — descriptive, used for display and variable referencing
- **agent** — the agent that executes this step (use `agent_find` to discover available agents)
- **prompt_template** — the instructions sent to the agent. Use:
  - `{{input}}` for the previous step's output
  - `{{var_name}}` for named variables from earlier steps
  - `{{param}}` for workflow-level input parameters

### Execution modes

| Mode | Use when |
|------|----------|
| `sequential` (default) | Steps run one after another |
| `fan_out` | One step spawns multiple parallel instances |
| `collect` | Gather results from fan_out into a single output |
| `conditional` | Branch based on a condition in the previous output |
| `loop` | Repeat a step until a condition is met |
| `wait` | Pause for a fixed duration |
| `approval` | Require human approval before continuing |

### Error handling

| error_mode | Behavior |
|------------|----------|
| `fail` (default) | Stop the workflow on error |
| `skip` | Continue to the next step |
| `retry` | Retry the step with backoff |

### Dependencies and variables

- **`depends_on`**: List step names this step depends on. When set, the workflow engine uses DAG (directed acyclic graph) execution instead of sequential.
- **`output_var`**: Store this step's output as a named variable. Later steps can reference it with `{{var_name}}`.

### Input parameters

Declare `input_schema` so callers know what to pass to `workflow_run`:

```json
[
  { "name": "repo_url", "type": "string", "description": "Repository URL to analyze", "required": true },
  { "name": "max_depth", "type": "number", "description": "Max analysis depth", "required": false }
]
```

## Examples

### Simple code review workflow

Ask the user which agents they have, then design:

```json
{
  "name": "code-review",
  "description": "Review code changes: style check, test run, summary",
  "steps": [
    {
      "name": "style-check",
      "agent": "code-reviewer",
      "prompt_template": "Check the code style and conventions in {{input}}. Report any violations.",
      "output_var": "style_check",
      "error_mode": "skip"
    },
    {
      "name": "run-tests",
      "agent": "test-runner",
      "prompt_template": "Run the test suite on {{input}}. Report failures.",
      "output_var": "test_results",
      "depends_on": ["style-check"],
      "error_mode": "fail"
    },
    {
      "name": "summarize",
      "agent": "assistant",
      "prompt_template": "Summarize the review results. Style check: {{style_check}}. Tests: {{test_results}}. Provide a verdict: approved or changes requested.",
      "depends_on": ["style-check", "run-tests"]
    }
  ],
  "input_schema": [
    { "name": "code_diff", "type": "string", "description": "Git diff or code to review", "required": true }
  ]
}
```

### Research briefing workflow

```json
{
  "name": "daily-briefing",
  "description": "Compile a daily research briefing on a topic",
  "steps": [
    {
      "name": "search",
      "agent": "researcher",
      "prompt_template": "Search for the latest news and developments about {{topic}} from the past 24 hours.",
      "output_var": "search",
      "timeout_secs": 300
    },
    {
      "name": "analyze",
      "agent": "analyst",
      "prompt_template": "Analyze these search results and extract the 5 most important developments. For each: headline, one-paragraph summary, and significance rating (1-5). Results: {{search}}",
      "output_var": "analysis"
    },
    {
      "name": "format",
      "agent": "assistant",
      "prompt_template": "Format this analysis into a clean daily briefing email. Include a subject line, executive summary, and the 5 developments with ratings. Analysis: {{analysis}}"
    }
  ],
  "input_schema": [
    { "name": "topic", "type": "string", "description": "Topic for the briefing", "required": true }
  ]
}
```

## Best practices

1. **Name workflows descriptively** — `code-review` not `wf1`
2. **Use `depends_on` for parallelism** — independent steps can run concurrently
3. **Use `output_var` for key results** — makes later steps cleaner
4. **Set appropriate `timeout_secs`** — longer for research, shorter for simple tasks
5. **Use `error_mode: skip` for non-critical steps** — keeps the workflow running
6. **Declare `input_schema`** — makes the workflow self-documenting
7. **Validate agent names** with `agent_find` before creating
8. **Start simple** — 2-3 steps is often enough; add complexity only when needed

## After creation

Once created, workflows appear in the dashboard and are available via:
- `workflow_list` — see all available workflows
- `workflow_describe` — view parameters and steps
- `workflow_run` — execute synchronously
- `workflow_start` — execute asynchronously (fire and forget)
