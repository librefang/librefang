import { describe, expect, it } from 'vitest'
import { getTranslation, rawTranslations, languages } from './i18n'

// Recursively walk an object and return dotted paths of primitive leaves.
// We use paths rather than flattening to objects so we can detect when a
// locale is missing an entire sub-block, not just individual keys.
function leafPaths(value: unknown, prefix = ''): string[] {
  if (Array.isArray(value)) {
    // Arrays: lock down only length so each locale has the same shape. We
    // allow arrays of strings/objects to have locale-specific content.
    return [`${prefix}[length=${value.length}]`]
  }
  if (value !== null && typeof value === 'object') {
    const out: string[] = []
    for (const key of Object.keys(value as Record<string, unknown>).sort()) {
      out.push(...leafPaths((value as Record<string, unknown>)[key], prefix ? `${prefix}.${key}` : key))
    }
    return out
  }
  return [prefix]
}

const EN_PATHS = new Set(leafPaths(rawTranslations.en))

describe('i18n completeness', () => {
  it('every declared locale has a raw translations entry', () => {
    for (const lang of languages) {
      expect(rawTranslations[lang.code], `missing translations for ${lang.code}`).toBeDefined()
    }
  })

  it('declares Ukrainian as a selectable locale', () => {
    expect(languages.some(lang => lang.code === 'uk')).toBe(true)
  })

  it('falls back to English when a locale is unknown', () => {
    expect(getTranslation('missing-locale').hero.title1).toBe(rawTranslations.en!.hero.title1)
  })

  it('falls back to English for keys missing in the selected locale', () => {
    const uk = getTranslation('uk')
    expect(uk.hero.title1).toBe('Агентна')
    expect(uk.common!.copy).toBe('Копіювати')
  })

  // Every locale must resolve to the full EN shape after fallback.
  // Raw locale objects may stay partial; getTranslation is the runtime contract.
  for (const lang of languages) {
    if (lang.code === 'en') continue
    it(`${lang.code} resolves missing keys from en`, () => {
      const paths = new Set(leafPaths(getTranslation(lang.code)))
      const missing: string[] = []
      for (const p of EN_PATHS) {
        if (!paths.has(p)) missing.push(p)
      }
      expect(missing, `${lang.code} missing: ${missing.slice(0, 8).join(', ')}`).toEqual([])
    })
  }
})
