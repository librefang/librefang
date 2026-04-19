#!/usr/bin/env npx tsx
// Build-time script: emit one SVG OG image per registry category so sharing
// /skills vs /channels links to Twitter/Slack shows a category-specific card
// instead of the single generic default image.
//
// Why SVG and not PNG? Every downstream consumer (Twitter cards, Slack link
// unfurls, Discord embeds, OpenGraph) accepts SVG as og:image, and SVGs are
// 1/50th the size of the equivalent PNG, live in the repo as text, and stay
// crisp on high-DPI displays. Run via `pnpm build` prebuild step.

import { writeFileSync, mkdirSync, readFileSync, existsSync } from 'node:fs'
import { join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = join(__dirname, '..', 'public', 'og')
const REGISTRY_JSON = join(__dirname, '..', 'public', 'registry.json')

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

interface RegistryItem { id: string; name: string; description?: string; icon?: string }

// Escape characters that would otherwise close the SVG attribute or
// embed arbitrary markup. OG text from the registry is user-controlled.
function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function renderItem(def: CategoryDef, item: RegistryItem): string {
  const name = esc((item.name || item.id).slice(0, 48))
  const desc = esc((item.description || def.subtitle).slice(0, 120))
  // Registry icons are now "lucide:<name>" tokens — can't render a React
  // component into a static SVG, so fall back to the category glyph.
  const rawIcon = item.icon && !item.icon.startsWith('lucide:') ? item.icon : def.icon
  const icon = esc(rawIcon)
  const id = esc(item.id)
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

  <text x="80" y="96" font-family="Arial, Helvetica, sans-serif" font-size="20" fill="#475569">librefang.ai / ${def.slug} / ${id}</text>

  <text x="80" y="220" font-family="Arial, Helvetica, sans-serif" font-size="28" font-weight="700" fill="${def.accent}" letter-spacing="2">${esc(def.title.toUpperCase())}</text>
  <text x="80" y="310" font-family="Arial, Helvetica, sans-serif" font-size="88" font-weight="900" fill="#ffffff" letter-spacing="-2">${name}</text>

  <foreignObject x="80" y="350" width="900" height="160">
    <div xmlns="http://www.w3.org/1999/xhtml" style="font-family:Arial,Helvetica,sans-serif;font-size:26px;line-height:1.4;color:#94a3b8;overflow:hidden;max-height:140px;">${desc}</div>
  </foreignObject>

  <text x="1050" y="340" font-family="Arial, Helvetica, sans-serif" font-size="360" fill="${def.accent}" opacity="0.18" text-anchor="middle">${icon}</text>

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
  let itemCount = 0
  if (existsSync(REGISTRY_JSON)) {
    const defBySlug = new Map(CATEGORIES.map(d => [d.slug, d]))
    try {
      const data = JSON.parse(readFileSync(REGISTRY_JSON, 'utf8')) as Record<string, RegistryItem[] | unknown>
      for (const def of CATEGORIES) {
        const arr = data[def.slug]
        if (!Array.isArray(arr)) continue
        const catDir = join(OUT_DIR, def.slug)
        mkdirSync(catDir, { recursive: true })
        // Registry data is user-contributed. Reject any id that isn't a
        // pure slug so a crafted entry can't write outside catDir via `..`
        // or an absolute path.
        const SLUG_RE = /^[a-z0-9][a-z0-9_-]*$/i
        for (const raw of arr as RegistryItem[]) {
          if (!raw || typeof raw.id !== 'string' || !SLUG_RE.test(raw.id)) continue
          const d = defBySlug.get(def.slug)!
          writeFileSync(join(catDir, `${raw.id}.svg`), renderItem(d, raw))
          itemCount++
        }
      }
    } catch (err) {
      console.warn('Could not read registry.json for per-item OGs:', err)
    }
  }
  console.log(`Wrote ${CATEGORIES.length} category + ${itemCount} item OG images to ${OUT_DIR}`)
}

if (import.meta.url === `file://${process.argv[1]}`) main()
