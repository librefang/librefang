import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  buildFeed,
  escapeCdata,
  escapeXml,
  generateFeed,
  parseEntries,
  renderEntry,
} from './gen-rss'

afterEach(() => vi.restoreAllMocks())

const SAMPLE = `# Changelog

Preamble text.

## [2026.4.15] - 2026-04-15

### Added

- First feature

## [2026.4.14] - 2026-04-14

### Fixed

- A bug

## [2026.4.13] - 2026-04-13

Nothing here.
`

describe('parseEntries', () => {
  it('returns entries in document order, newest first', () => {
    const out = parseEntries(SAMPLE, 10)
    expect(out.map(e => e.version)).toEqual(['2026.4.15', '2026.4.14', '2026.4.13'])
    expect(out[0]!.date).toBe('2026-04-15')
  })

  it('respects max', () => {
    expect(parseEntries(SAMPLE, 2)).toHaveLength(2)
  })

  it('body captures the section under the heading', () => {
    const out = parseEntries(SAMPLE, 1)
    expect(out[0]!.body).toContain('### Added')
    expect(out[0]!.body).toContain('First feature')
    expect(out[0]!.body).not.toContain('2026.4.14')
  })

  it('returns empty on no matches', () => {
    expect(parseEntries('# Just a heading', 5)).toEqual([])
  })

  it('warns on non-versioned h2 headings and keeps them out of entry bodies', () => {
    const warn = vi.fn()
    const entries = parseEntries(
      '## [1.0.0] - 2026-01-01\nrelease body\n## [Unreleased]\ndraft body',
      5,
      warn,
    )

    expect(entries).toHaveLength(1)
    expect(entries[0]?.body).toBe('release body')
    expect(warn).toHaveBeenCalledWith('Skipping non-versioned changelog heading: ## [Unreleased]')
  })
})

describe('escapeXml', () => {
  it('escapes the five xml entities', () => {
    expect(escapeXml(`a & b < c > d "e"`)).toBe('a &amp; b &lt; c &gt; d &quot;e&quot;')
  })
})

describe('renderEntry', () => {
  it('wraps the body in CDATA', () => {
    const r = renderEntry({ version: '1.0.0', date: '2026-01-01', body: '## body' })
    expect(r).toContain('<![CDATA[## body]]>')
    expect(r).toContain('<updated>2026-01-01T00:00:00Z</updated>')
  })

  it('splits CDATA terminators without changing the text payload', () => {
    const body = 'before ]]> after'
    const rendered = renderEntry({ version: '1.0.0', date: '2026-01-01', body })

    expect(escapeCdata(body)).toBe('before ]]]]><![CDATA[> after')
    expect(rendered).toContain('<![CDATA[before ]]]]><![CDATA[> after]]>')
  })

  it('uses the configured site for entry ids and links', () => {
    const rendered = renderEntry(
      { version: '1.0.0', date: '2026-01-01', body: '' },
      'https://example.com/',
    )

    expect(rendered).toContain('<id>https://example.com/changelog/#1-0-0</id>')
    expect(rendered).toContain('href="https://example.com/changelog/#1-0-0"')
  })
})

describe('buildFeed', () => {
  it('produces valid-looking Atom with N entries', () => {
    const { xml, entries } = buildFeed(SAMPLE, { max: 3 })
    expect(entries).toHaveLength(3)
    expect(xml).toMatch(/^<\?xml version="1\.0"/)
    expect(xml).toContain('<feed xmlns="http://www.w3.org/2005/Atom">')
    expect(xml.match(/<entry>/g)).toHaveLength(3)
    expect(xml).toContain('<updated>2026-04-15T00:00:00Z</updated>')
  })

  it('custom site/author are threaded through and XML-escaped', () => {
    const { xml } = buildFeed(SAMPLE, { site: 'https://example.com', author: 'X <x@y.z>', max: 1 })
    expect(xml).toContain('https://example.com/feed.xml')
    expect(xml).toContain('<author><name>X</name><email>x@y.z</email></author>')
    expect(xml).toContain('https://example.com/changelog/#2026-4-15')
  })

  it('renders structured author fields according to Atom', () => {
    const { xml } = buildFeed(SAMPLE, {
      author: { name: 'R&D <team>', email: 'feed&alerts@example.com' },
      max: 1,
    })

    expect(xml).toContain(
      '<author><name>R&amp;D &lt;team&gt;</name><email>feed&amp;alerts@example.com</email></author>',
    )
  })

  it('empty changelog yields a feed with zero entries', () => {
    const { xml, entries } = buildFeed('# Changelog\n', { max: 5 })
    expect(entries).toHaveLength(0)
    expect(xml).toContain('<feed')
    expect(xml.match(/<entry>/g)).toBeNull()
  })
})

describe('generateFeed', () => {
  it('adds path context to changelog read failures', () => {
    const missing = join(tmpdir(), 'librefang-rss-missing', 'CHANGELOG.md')

    expect(() => generateFeed(missing)).toThrow(`Unable to read changelog at ${missing}:`)
  })

  it('adds path context to output write failures', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    const dir = mkdtempSync(join(tmpdir(), 'librefang-rss-'))
    const changelog = join(dir, 'CHANGELOG.md')
    writeFileSync(changelog, SAMPLE)

    try {
      expect(() => generateFeed(changelog, dir)).toThrow(
        `Unable to write Atom feed at ${dir}:`,
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
