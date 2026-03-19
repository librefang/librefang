import type { Node, Edge } from "@xyflow/react";

export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  category: string;
  icon: string;
  nodes: Node[];
  edges: Edge[];
  steps?: Array<{ name: string; prompt: string }>;
}

export const workflowTemplates: WorkflowTemplate[] = [
  {
    id: "daily-summary",
    name: "workflows.template_details.daily_summary",
    description: "workflows.template_details.daily_summary_desc",
    category: "automation",
    icon: "📅",
    nodes: [
      {
        id: "start-1",
        type: "custom",
        position: { x: 100, y: 100 },
        data: { label: "workflows.nodes.daily", nodeType: "schedule", description: "workflows.nodes.daily_desc" }
      },
      {
        id: "agent-1",
        type: "custom",
        position: { x: 100, y: 250 },
        data: { label: "workflows.nodes.summarizer", nodeType: "agent", description: "workflows.nodes.summarizer_desc" }
      },
      {
        id: "channel-1",
        type: "custom",
        position: { x: 100, y: 400 },
        data: { label: "workflows.nodes.notify", nodeType: "channel", description: "workflows.nodes.notify_desc" }
      },
      {
        id: "end-1",
        type: "custom",
        position: { x: 100, y: 550 },
        data: { label: "workflows.nodes.complete", nodeType: "end", description: "workflows.nodes.complete_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "start-1", target: "agent-1" },
      { id: "e2", source: "agent-1", target: "channel-1" },
      { id: "e3", source: "channel-1", target: "end-1" }
    ],
    steps: [
      { name: "trigger", prompt: "Run daily at midnight" },
      { name: "collect", prompt: "Gather all events and activities from the past day" },
      { name: "summarize", prompt: "Create a concise summary of key events" },
      { name: "notify", prompt: "Send summary to configured notification channel" }
    ]
  },
  {
    id: "news-digest",
    name: "workflows.template_details.news_digest",
    description: "workflows.template_details.news_digest_desc",
    category: "information",
    icon: "📰",
    nodes: [
      {
        id: "start-2",
        type: "custom",
        position: { x: 100, y: 100 },
        data: { label: "workflows.nodes.scheduled", nodeType: "schedule", description: "workflows.nodes.scheduled_desc" }
      },
      {
        id: "webhook-1",
        type: "custom",
        position: { x: 100, y: 250 },
        data: { label: "workflows.nodes.fetch", nodeType: "webhook", description: "workflows.nodes.fetch_desc" }
      },
      {
        id: "agent-2",
        type: "custom",
        position: { x: 100, y: 400 },
        data: { label: "workflows.nodes.analyze", nodeType: "agent", description: "workflows.nodes.analyze_desc" }
      },
      {
        id: "channel-2",
        type: "custom",
        position: { x: 100, y: 550 },
        data: { label: "workflows.nodes.deliver", nodeType: "channel", description: "workflows.nodes.deliver_desc" }
      },
      {
        id: "end-2",
        type: "custom",
        position: { x: 100, y: 700 },
        data: { label: "workflows.nodes.done", nodeType: "end", description: "workflows.nodes.done_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "start-2", target: "webhook-1" },
      { id: "e2", source: "webhook-1", target: "agent-2" },
      { id: "e3", source: "agent-2", target: "channel-2" },
      { id: "e4", source: "channel-2", target: "end-2" }
    ],
    steps: [
      { name: "schedule", prompt: "Run every 6 hours" },
      { name: "fetch", prompt: "Fetch latest news from configured sources" },
      { name: "analyze", prompt: "Analyze and extract key information" },
      { name: "deliver", prompt: "Format and send digest to channel" }
    ]
  },
  {
    id: "system-health",
    name: "workflows.template_details.system_health",
    description: "workflows.template_details.system_health_desc",
    category: "monitoring",
    icon: "🩺",
    nodes: [
      {
        id: "start-3",
        type: "custom",
        position: { x: 100, y: 100 },
        data: { label: "workflows.nodes.trigger", nodeType: "schedule", description: "workflows.nodes.trigger_desc" }
      },
      {
        id: "agent-3",
        type: "custom",
        position: { x: 100, y: 250 },
        data: { label: "workflows.nodes.check", nodeType: "agent", description: "workflows.nodes.check_desc" }
      },
      {
        id: "condition-1",
        type: "custom",
        position: { x: 100, y: 400 },
        data: { label: "workflows.nodes.healthy", nodeType: "condition", description: "workflows.nodes.healthy_desc" }
      },
      {
        id: "channel-3",
        type: "custom",
        position: { x: 50, y: 550 },
        data: { label: "workflows.nodes.alert", nodeType: "channel", description: "workflows.nodes.alert_desc" }
      },
      {
        id: "end-3",
        type: "custom",
        position: { x: 200, y: 550 },
        data: { label: "workflows.nodes.ok", nodeType: "end", description: "workflows.nodes.ok_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "start-3", target: "agent-3" },
      { id: "e2", source: "agent-3", target: "condition-1" },
      { id: "e3", source: "condition-1", target: "channel-3", label: "Alert" },
      { id: "e4", source: "condition-1", target: "end-3", label: "OK" }
    ],
    steps: [
      { name: "trigger", prompt: "Run every hour" },
      { name: "check", prompt: "Query system status, memory, CPU, agents" },
      { name: "condition", prompt: "Check if all metrics are healthy" },
      { name: "alert", prompt: "Send alert if any metric is critical" }
    ]
  },
  {
    id: "multi-agent",
    name: "workflows.template_details.multi_agent",
    description: "workflows.template_details.multi_agent_desc",
    category: "advanced",
    icon: "🤖",
    nodes: [
      {
        id: "start-4",
        type: "custom",
        position: { x: 200, y: 50 },
        data: { label: "workflows.nodes.input", nodeType: "webhook", description: "workflows.nodes.input_desc" }
      },
      {
        id: "parallel-1",
        type: "custom",
        position: { x: 200, y: 200 },
        data: { label: "workflows.nodes.parallel", nodeType: "parallel", description: "workflows.nodes.parallel_desc" }
      },
      {
        id: "agent-4a",
        type: "custom",
        position: { x: 50, y: 350 },
        data: { label: "workflows.nodes.researcher", nodeType: "agent", description: "workflows.nodes.researcher_desc" }
      },
      {
        id: "agent-4b",
        type: "custom",
        position: { x: 350, y: 350 },
        data: { label: "workflows.nodes.writer", nodeType: "agent", description: "workflows.nodes.writer_desc" }
      },
      {
        id: "agent-5",
        type: "custom",
        position: { x: 200, y: 500 },
        data: { label: "workflows.nodes.coordinator", nodeType: "agent", description: "workflows.nodes.coordinator_desc" }
      },
      {
        id: "end-4",
        type: "custom",
        position: { x: 200, y: 650 },
        data: { label: "workflows.nodes.response", nodeType: "end", description: "workflows.nodes.response_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "start-4", target: "parallel-1" },
      { id: "e2", source: "parallel-1", target: "agent-4a" },
      { id: "e3", source: "parallel-1", target: "agent-4b" },
      { id: "e4", source: "agent-4a", target: "agent-5" },
      { id: "e5", source: "agent-4b", target: "agent-5" },
      { id: "e6", source: "agent-5", target: "end-4" }
    ],
    steps: [
      { name: "input", prompt: "Receive task via webhook" },
      { name: "parallel", prompt: "Execute multiple agents in parallel" },
      { name: "research", prompt: "Research agent gathers information" },
      { name: "write", prompt: "Writer agent creates content" },
      { name: "coordinate", prompt: "Coordinator merges results" }
    ]
  }
];
