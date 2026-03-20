import type { Node, Edge } from "@xyflow/react";

export interface WorkflowTemplate {
  id: string;
  name: string;
  description: string;
  category: string;
  icon: string;
  nodes: Node[];
  edges: Edge[];
}

export const workflowTemplates: WorkflowTemplate[] = [
  // ── 1. 内容创作流水线 ──────────────────────────────
  {
    id: "content-pipeline",
    name: "workflows.template_details.content_pipeline",
    description: "workflows.template_details.content_pipeline_desc",
    category: "creation",
    icon: "FileText",
    nodes: [
      {
        id: "n1", type: "custom", position: { x: 50, y: 0 },
        data: { label: "workflows.nodes.topic_input", nodeType: "start", description: "workflows.nodes.topic_input_desc" }
      },
      {
        id: "n2", type: "custom", position: { x: 50, y: 80 },
        data: {
          label: "workflows.nodes.researcher", nodeType: "agent",
          description: "workflows.nodes.researcher_desc",
          prompt: "You are a research assistant. Research the following topic thoroughly and provide: 1) Background and context 2) Key facts and data points 3) Different perspectives on the topic 4) Recent developments. Be specific and detailed.\n\nTopic: {{input}}"
        }
      },
      {
        id: "n3", type: "custom", position: { x: 50, y: 160 },
        data: {
          label: "workflows.nodes.writer", nodeType: "agent",
          description: "workflows.nodes.writer_desc",
          prompt: "You are a professional writer. Based on the research below, write a well-structured article (800-1200 words). Include: a compelling headline, an engaging introduction, clear sections with subheadings, and a strong conclusion. Write in a clear, accessible style.\n\nResearch:\n{{input}}"
        }
      },
      {
        id: "n4", type: "custom", position: { x: 50, y: 240 },
        data: {
          label: "workflows.nodes.editor", nodeType: "agent",
          description: "workflows.nodes.editor_desc",
          prompt: "You are a senior editor. Review and polish the following article. Fix any grammar issues, improve flow, tighten wordy sentences, and ensure factual consistency. Output the final polished version only, no commentary.\n\nDraft:\n{{input}}"
        }
      },
      {
        id: "n5", type: "custom", position: { x: 50, y: 320 },
        data: { label: "workflows.nodes.complete", nodeType: "end", description: "workflows.nodes.complete_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "n1", target: "n2" },
      { id: "e2", source: "n2", target: "n3" },
      { id: "e3", source: "n3", target: "n4" },
      { id: "e4", source: "n4", target: "n5" }
    ]
  },

  // ── 2. 翻译润色 ──────────────────────────────────
  {
    id: "translate-polish",
    name: "workflows.template_details.translate_polish",
    description: "workflows.template_details.translate_polish_desc",
    category: "language",
    icon: "Bot",
    nodes: [
      {
        id: "n1", type: "custom", position: { x: 50, y: 0 },
        data: { label: "workflows.nodes.source_text", nodeType: "start", description: "workflows.nodes.source_text_desc" }
      },
      {
        id: "n2", type: "custom", position: { x: 50, y: 80 },
        data: {
          label: "workflows.nodes.translator", nodeType: "agent",
          description: "workflows.nodes.translator_desc",
          prompt: "You are an expert translator. Translate the following text. Auto-detect the source language: if it's Chinese, translate to English; if it's English, translate to Chinese; for other languages, translate to both Chinese and English. Preserve the original tone, style, and formatting.\n\nText:\n{{input}}"
        }
      },
      {
        id: "n3", type: "custom", position: { x: 50, y: 160 },
        data: {
          label: "workflows.nodes.reviewer", nodeType: "agent",
          description: "workflows.nodes.reviewer_desc",
          prompt: "You are a bilingual language expert. Review the following translation for accuracy and naturalness. Check for: 1) Mistranslations or meaning shifts 2) Awkward phrasing that sounds translated 3) Cultural context issues. Output the improved final translation only.\n\nTranslation to review:\n{{input}}"
        }
      },
      {
        id: "n4", type: "custom", position: { x: 50, y: 240 },
        data: { label: "workflows.nodes.complete", nodeType: "end", description: "workflows.nodes.complete_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "n1", target: "n2" },
      { id: "e2", source: "n2", target: "n3" },
      { id: "e3", source: "n3", target: "n4" }
    ]
  },

  // ── 3. 头脑风暴 ──────────────────────────────────
  {
    id: "brainstorm",
    name: "workflows.template_details.brainstorm",
    description: "workflows.template_details.brainstorm_desc",
    category: "thinking",
    icon: "Activity",
    nodes: [
      {
        id: "n1", type: "custom", position: { x: 50, y: 0 },
        data: { label: "workflows.nodes.challenge", nodeType: "start", description: "workflows.nodes.challenge_desc" }
      },
      {
        id: "n2", type: "custom", position: { x: 50, y: 80 },
        data: {
          label: "workflows.nodes.idea_gen", nodeType: "agent",
          description: "workflows.nodes.idea_gen_desc",
          prompt: "You are a creative strategist. For the following challenge, generate 10 diverse solution ideas. Include: 3 conventional approaches, 4 creative approaches, and 3 bold/unconventional approaches. For each idea, give a one-line title and a 2-sentence explanation.\n\nChallenge: {{input}}"
        }
      },
      {
        id: "n3", type: "custom", position: { x: 50, y: 160 },
        data: {
          label: "workflows.nodes.evaluator", nodeType: "agent",
          description: "workflows.nodes.evaluator_desc",
          prompt: "You are a critical analyst. Evaluate each idea below on three criteria: Feasibility (1-5), Impact (1-5), Originality (1-5). Pick the top 3 ideas and for each provide: why it's promising, key risks, and concrete first steps to implement it.\n\nIdeas:\n{{input}}"
        }
      },
      {
        id: "n4", type: "custom", position: { x: 50, y: 240 },
        data: {
          label: "workflows.nodes.action_plan", nodeType: "agent",
          description: "workflows.nodes.action_plan_desc",
          prompt: "You are a project planner. Take the top-rated idea below and create a concrete action plan: 1) Clear goal statement 2) 5-step implementation roadmap with timelines 3) Required resources 4) Success metrics 5) Potential obstacles and mitigation strategies.\n\nEvaluation:\n{{input}}"
        }
      },
      {
        id: "n5", type: "custom", position: { x: 50, y: 320 },
        data: { label: "workflows.nodes.complete", nodeType: "end", description: "workflows.nodes.complete_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "n1", target: "n2" },
      { id: "e2", source: "n2", target: "n3" },
      { id: "e3", source: "n3", target: "n4" },
      { id: "e4", source: "n4", target: "n5" }
    ]
  },

  // ── 4. 周报生成器 ─────────────────────────────────
  {
    id: "weekly-report",
    name: "workflows.template_details.weekly_report",
    description: "workflows.template_details.weekly_report_desc",
    category: "business",
    icon: "Calendar",
    nodes: [
      {
        id: "n1", type: "custom", position: { x: 50, y: 0 },
        data: { label: "workflows.nodes.work_notes", nodeType: "start", description: "workflows.nodes.work_notes_desc" }
      },
      {
        id: "n2", type: "custom", position: { x: 50, y: 80 },
        data: {
          label: "workflows.nodes.organizer", nodeType: "agent",
          description: "workflows.nodes.organizer_desc",
          prompt: "You are an executive assistant. Organize the following raw work notes into structured categories: 1) Completed tasks 2) In-progress items 3) Blockers/Issues 4) Key decisions made. Extract and list each item clearly, even if the notes are messy.\n\nRaw notes:\n{{input}}"
        }
      },
      {
        id: "n3", type: "custom", position: { x: 50, y: 160 },
        data: {
          label: "workflows.nodes.report_writer", nodeType: "agent",
          description: "workflows.nodes.report_writer_desc",
          prompt: "You are a professional report writer. Transform the organized items below into a polished weekly report with these sections:\n\n## This Week's Highlights\n(top 3 achievements)\n\n## Progress Details\n(organized by project/area)\n\n## Challenges & Solutions\n(any blockers and how they were addressed)\n\n## Next Week's Priorities\n(based on in-progress items)\n\nKeep it concise and professional.\n\nOrganized items:\n{{input}}"
        }
      },
      {
        id: "n4", type: "custom", position: { x: 50, y: 240 },
        data: { label: "workflows.nodes.complete", nodeType: "end", description: "workflows.nodes.complete_desc" }
      }
    ],
    edges: [
      { id: "e1", source: "n1", target: "n2" },
      { id: "e2", source: "n2", target: "n3" },
      { id: "e3", source: "n3", target: "n4" }
    ]
  }
];
