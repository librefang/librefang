#!/usr/bin/env npx tsx
// Build-time script: emit one SVG OG image per registry category so sharing
// /skills vs /channels links to Twitter/Slack shows a category-specific card
// instead of the single generic default image.
//
// Why SVG and not PNG? Every downstream consumer (Twitter cards, Slack link
// unfurls, Discord embeds, OpenGraph) accepts SVG as og:image, and SVGs are
// 1/50th the size of the equivalent PNG, live in the repo as text, and stay
// crisp on high-DPI displays. Run via `pnpm build` prebuild step.

import { writeFileSync, mkdirSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = join(__dirname, '..', 'public', 'og')

export interface CategoryDef {
  slug: string
  title: string
  subtitle: string
  accent: string       // primary accent colour for the glow and "$" prompt
  icon: string         // big glyph top-right
}

// Colour palette chosen so each category is distinguishable at a glance in a
// Slack/Twitter feed. Accents pulled from the existing tailwind palette.
export const CATEGORIES: CategoryDef[] = [
  { slug: 'skills',       title: 'Skills',       subtitle: '60 pluggable tool bundles', accent: '#f59e0b', icon: '⚡' },
  { slug: 'hands',        title: 'Hands',        subtitle: 'Autonomous capability units', accent: '#06b6d4', icon: '◉' },
  { slug: 'agents',       title: 'Agents',       subtitle: 'Pre-built agent templates', accent: '#a78bfa', icon: '◆' },
  { slug: 'providers',    title: 'Providers',    subtitle: 'LLM provider adapters', accent: '#34d399', icon: '▲' },
  { slug: 'workflows',    title: 'Workflows',    subtitle: 'Multi-step orchestrations', accent: '#f87171', icon: '↠' },
  { slug: 'channels',     title: 'Channels',     subtitle: 'Messaging platform adapters', accent: '#60a5fa', icon: '✉' },
  { slug: 'plugins',      title: 'Plugins',      subtitle: 'Runtime extensions', accent: '#e879f9', icon: '✦' },
  { slug: 'mcp',          title: 'MCP Servers',  subtitle: 'Model Context Protocol', accent: '#fbbf24', icon: '⚙' },
  { slug: 'integrations', title: 'Integrations', subtitle: 'First-party integrations', accent: '#22d3ee', icon: '⇌' },
]

export function render(def: CategoryDef): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 630" width="1200" height="630">
  <rect width="1200" height="630" fill="#070b14"/>

  <defs>
    <pattern id="grid" width="60" height="60" patternUnits="userSpaceOnUse">
      <path d="M 60 0 L 0 0 0 60" fill="none" stroke="#0f1729" stroke-width="0.5"/>
    </pattern>
    <radialGradient id="glow" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="${def.accent}" stop-opacity="0.28"/>
      <stop offset="100%" stop-color="${def.accent}" stop-opacity="0"/>
    </radialGradient>
  </defs>

  <rect width="1200" height="630" fill="url(#grid)" opacity="0.6"/>
  <circle cx="980" cy="160" r="320" fill="url(#glow)"/>

  <!-- Top-left brand -->
  <text x="80" y="96" font-family="Arial, Helvetica, sans-serif" font-size="20" fill="#475569">librefang.ai / registry</text>

  <!-- Category title -->
  <text x="80" y="260" font-family="Arial, Helvetica, sans-serif" font-size="112" font-weight="900" fill="#ffffff" letter-spacing="-3">${def.title}</text>

  <!-- Subtitle -->
  <text x="80" y="320" font-family="Arial, Helvetica, sans-serif" font-size="30" fill="${def.accent}">${def.subtitle}</text>

  <!-- Pills — a fake set of tags to hint at "a collection of things" -->
  <g font-family="monospace" font-size="16" fill="#64748b">
    <rect x="80" y="380" width="110" height="36" rx="4" fill="#0d1424" stroke="#1e293b"/>
    <text x="135" y="404" text-anchor="middle">production</text>
    <rect x="200" y="380" width="90" height="36" rx="4" fill="#0d1424" stroke="#1e293b"/>
    <text x="245" y="404" text-anchor="middle">open source</text>
    <rect x="300" y="380" width="74" height="36" rx="4" fill="#0d1424" stroke="#1e293b"/>
    <text x="337" y="404" text-anchor="middle">Rust</text>
  </g>

  <!-- Big icon top-right -->
  <text x="1050" y="340" font-family="Arial, Helvetica, sans-serif" font-size="360" fill="${def.accent}" opacity="0.18" text-anchor="middle">${def.icon}</text>

  <!-- Bottom accent line -->
  <rect x="80" y="560" width="160" height="3" rx="1.5" fill="${def.accent}" opacity="0.8"/>
  <text x="80" y="594" font-family="Arial, Helvetica, sans-serif" font-size="18" fill="#94a3b8">LibreFang · the agent operating system</text>
</svg>
`
}

function main() {
  mkdirSync(OUT_DIR, { recursive: true })
  for (const def of CATEGORIES) {
    writeFileSync(join(OUT_DIR, `${def.slug}.svg`), render(def))
  }
  console.log(`Wrote ${CATEGORIES.length} OG images to ${OUT_DIR}`)
}

if (import.meta.url === `file://${process.argv[1]}`) main()
