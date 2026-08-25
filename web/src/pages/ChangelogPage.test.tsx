import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  detectReleaseType,
  formatDate,
  linkify,
  parseChanges,
  parseReleasesPayload,
} from './ChangelogPage'

describe('changelog parsing', () => {
  it('uses the GitHub prerelease flag when the tag has no known suffix', () => {
    expect(detectReleaseType('v1.0.0', false)).toBe('stable')
    expect(detectReleaseType('v1.0.0-rc.1', true)).toBe('rc')
    expect(detectReleaseType('v1.0.0-beta.1', true)).toBe('beta')
    expect(detectReleaseType('v1.0.0-alpha.1', true)).toBe('rc')
  })

  it('parses categorized release notes', () => {
    const changes = parseChanges('### Added\n- New dashboard\n### Fixed\n- Crash on startup')

    expect(changes.get('feature')?.map((change) => change.text)).toEqual(['New dashboard'])
    expect(changes.get('fix')?.map((change) => change.text)).toEqual(['Crash on startup'])
  })

  it('accepts valid release payloads and rejects malformed responses', () => {
    const release = {
      id: 1,
      tag_name: 'v1.0.0',
      name: null,
      body: null,
      html_url: 'https://github.com/librefang/librefang/releases/tag/v1.0.0',
      published_at: '2026-08-17T00:00:00Z',
      prerelease: false,
      draft: false,
      assets: [
        {
          name: 'librefang.tar.gz',
          download_count: 4,
          browser_download_url: 'https://example.com/librefang.tar.gz',
        },
      ],
    }

    expect(parseReleasesPayload([release])).toEqual([release])
    expect(() => parseReleasesPayload([{ ...release, assets: null }])).toThrow(
      'Invalid release response from stats.librefang.ai',
    )
  })
})

describe('changelog rendering helpers', () => {
  it('escapes release text while linking issue and user references', () => {
    const markup = renderToStaticMarkup(<span>{linkify('<img src=x onerror=alert(1)> #123 @octocat')}</span>)

    expect(markup).toContain('&lt;img src=x onerror=alert(1)&gt;')
    expect(markup).not.toContain('<img')
    expect(markup).toContain('href="https://github.com/librefang/librefang/issues/123"')
    expect(markup).toContain('href="https://github.com/octocat"')
  })

  it('does not link embedded or entity-prefixed references', () => {
    const markup = renderToStaticMarkup(<span>{linkify('word#123 &#456')}</span>)

    expect(markup).not.toContain('<a')
  })

  it('formats missing publication dates as empty text', () => {
    expect(formatDate(null)).toBe('')
  })
})
