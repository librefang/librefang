#!/usr/bin/env npx tsx
// Build-time script: emit /feed.xml from CHANGELOG.md so readers can subscribe
// via RSS. Parses versioned h2 headings (## [X.Y.Z] - YYYY-MM-DD) as entries.

import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const CHANGELOG = join(__dirname, '..', '..', 'CHANGELOG.md')
const OUT = join(__dirname, '..', 'public', 'feed.xml')
const SITE = 'https://librefang.ai'
const AUTHOR = { name: 'LibreFang', email: 'noreply@librefang.ai' }

export interface Entry {
  version: string
  date: string
  body: string
}

export interface FeedAuthor {
  name: string
  email?: string
}

// Parse top-level versioned sections until we hit the next h2 or end.
export function parseEntries(
  md: string,
  max: number,
  warn: (message: string) => void = console.warn,
): Entry[] {
  const headings: {
    index: number
    length: number
    version?: string
    date?: string
  }[] = []
  const headingRe = /^##\s+(.+?)\s*$/gm
  let match: RegExpExecArray | null
  while ((match = headingRe.exec(md)) !== null) {
    const version = match[1]!.match(/^\[([^\]]+)\]\s*-\s*(\d{4}-\d{2}-\d{2})$/)
    headings.push({
      index: match.index,
      length: match[0].length,
      version: version?.[1],
      date: version?.[2],
    })
    if (!version) warn(`Skipping non-versioned changelog heading: ${match[0]}`)
  }

  const out: Entry[] = []
  for (let i = 0; i < headings.length && out.length < max; i++) {
    const current = headings[i]!
    if (!current.version || !current.date) continue
    const start = current.index + current.length
    const end = i + 1 < headings.length ? headings[i + 1]!.index : md.length
    out.push({
      version: current.version,
      date: current.date,
      body: md.slice(start, end).trim(),
    })
  }
  return out
}

export function escapeXml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

export function escapeCdata(s: string): string {
  return s.replace(/]]>/g, ']]]]><![CDATA[>')
}

function normalizeAuthor(author: string | FeedAuthor): FeedAuthor {
  if (typeof author !== 'string') return author
  const match = author.match(/^(.+?)\s*<([^<>]+)>$/)
  if (!match) return { name: author }
  return { name: match[1]!.trim(), email: match[2]!.trim() }
}

function renderAuthor(author: FeedAuthor): string {
  const email = author.email ? `<email>${escapeXml(author.email)}</email>` : ''
  return `<author><name>${escapeXml(author.name)}</name>${email}</author>`
}

// Render body Markdown to a plain-text summary. Full HTML conversion would be
// overkill; keep the Markdown intact inside CDATA so feed readers render it.
export function renderEntry(e: Entry, site = SITE): string {
  const base = site.replace(/\/+$/, '')
  const url = `${base}/changelog/#${e.version.replace(/\./g, '-')}`
  const escapedUrl = escapeXml(url)
  return `    <entry>
      <id>${escapedUrl}</id>
      <title>LibreFang ${escapeXml(e.version)}</title>
      <link href="${escapedUrl}" />
      <updated>${e.date}T00:00:00Z</updated>
      <summary type="text">${escapeXml(e.version)} — ${escapeXml(e.date)}</summary>
      <content type="text"><![CDATA[${escapeCdata(e.body)}]]></content>
    </entry>`
}

export function buildFeed(
  md: string,
  opts: {
    site?: string
    author?: string | FeedAuthor
    max?: number
    warn?: (message: string) => void
  } = {},
): { xml: string; entries: Entry[] } {
  const site = (opts.site ?? SITE).replace(/\/+$/, '')
  const author = normalizeAuthor(opts.author ?? AUTHOR)
  const max = opts.max ?? 30
  const entries = parseEntries(md, max, opts.warn)
  const latest = entries[0]?.date ?? new Date().toISOString().slice(0, 10)
  const escapedSite = escapeXml(site)
  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>LibreFang Changelog</title>
  <link href="${escapedSite}/feed.xml" rel="self" />
  <link href="${escapedSite}/changelog/" />
  <id>${escapedSite}/feed.xml</id>
  <updated>${latest}T00:00:00Z</updated>
  ${renderAuthor(author)}
${entries.map((entry) => renderEntry(entry, site)).join('\n')}
</feed>
`
  return { xml, entries }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

export function generateFeed(changelogPath = CHANGELOG, outPath = OUT) {
  let md: string
  try {
    md = readFileSync(changelogPath, 'utf-8')
  } catch (error) {
    throw new Error(`Unable to read changelog at ${changelogPath}: ${errorMessage(error)}`)
  }

  const { xml, entries } = buildFeed(md)
  if (entries.length === 0) console.warn('No changelog entries matched — feed will be empty.')
  try {
    mkdirSync(dirname(outPath), { recursive: true })
    writeFileSync(outPath, xml)
  } catch (error) {
    throw new Error(`Unable to write Atom feed at ${outPath}: ${errorMessage(error)}`)
  }
  console.log(`Wrote ${entries.length} entries to ${outPath}`)
  return { xml, entries }
}

const entrypoint = process.argv[1]
if (entrypoint && import.meta.url === pathToFileURL(resolve(entrypoint)).href) {
  try {
    generateFeed()
  } catch (error) {
    console.error(`RSS generation failed: ${errorMessage(error)}`)
    process.exitCode = 1
  }
}
