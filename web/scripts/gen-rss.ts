#!/usr/bin/env npx tsx
// Build-time script: emit /feed.xml from CHANGELOG.md so readers can subscribe
// via RSS. Parses versioned h2 headings (## [X.Y.Z] - YYYY-MM-DD) as entries.

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CHANGELOG = join(__dirname, '..', '..', 'CHANGELOG.md')
const OUT = join(__dirname, '..', 'public', 'feed.xml')
const SITE = 'https://librefang.ai'
const AUTHOR = 'LibreFang <noreply@librefang.ai>'

interface Entry {
  version: string
  date: string
  body: string
}

// Parse top-level versioned sections until we hit the next version or end.
function parseEntries(md: string, max: number): Entry[] {
  const out: Entry[] = []
  // Match "## [2026.4.15] - 2026-04-15"
  const re = /^##\s+\[([^\]]+)\]\s*-\s*(\d{4}-\d{2}-\d{2})\s*$/gm
  const heads: { match: RegExpExecArray; version: string; date: string }[] = []
  let m
  while ((m = re.exec(md)) !== null) {
    heads.push({ match: m, version: m[1]!, date: m[2]! })
    if (heads.length >= max * 2) break // safety cap on scans
  }
  for (let i = 0; i < heads.length && out.length < max; i++) {
    const cur = heads[i]!
    const start = cur.match.index + cur.match[0].length
    const end = i + 1 < heads.length ? heads[i + 1]!.match.index : md.length
    const body = md.slice(start, end).trim()
    out.push({ version: cur.version, date: cur.date, body })
  }
  return out
}

function escapeXml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

// Render body Markdown to a plain-text summary. Full HTML conversion would be
// overkill; keep the Markdown intact inside CDATA so feed readers render it.
function renderEntry(e: Entry): string {
  const url = `${SITE}/changelog/#${e.version.replace(/\./g, '-')}`
  return `    <entry>
      <id>${url}</id>
      <title>LibreFang ${escapeXml(e.version)}</title>
      <link href="${url}" />
      <updated>${e.date}T00:00:00Z</updated>
      <summary type="text">${escapeXml(e.version)} — ${escapeXml(e.date)}</summary>
      <content type="text"><![CDATA[${e.body}]]></content>
    </entry>`
}

function main() {
  const md = readFileSync(CHANGELOG, 'utf-8')
  const entries = parseEntries(md, 30)
  if (entries.length === 0) {
    console.warn('No changelog entries matched — feed will be empty.')
  }
  const latest = entries[0]?.date ?? new Date().toISOString().slice(0, 10)
  const feed = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>LibreFang Changelog</title>
  <link href="${SITE}/feed.xml" rel="self" />
  <link href="${SITE}/changelog/" />
  <id>${SITE}/feed.xml</id>
  <updated>${latest}T00:00:00Z</updated>
  <author><name>${AUTHOR}</name></author>
${entries.map(renderEntry).join('\n')}
</feed>
`
  mkdirSync(dirname(OUT), { recursive: true })
  writeFileSync(OUT, feed)
  console.log(`Wrote ${entries.length} entries to ${OUT}`)
}

main()
