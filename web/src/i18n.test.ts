import { describe, expect, it } from 'vitest'
import { translations, languages } from './i18n'

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

const EN_PATHS = new Set(leafPaths(translations.en))

describe('i18n completeness', () => {
  it('every declared locale has a translations entry', () => {
    for (const lang of languages) {
      expect(translations[lang.code], `missing translations for ${lang.code}`).toBeDefined()
    }
  })

  // For every non-en locale, every EN path must also exist. Optional fields
  // (?) are allowed to be missing in types, but if EN declares them, the
  // other locales should too.
  for (const lang of languages) {
    if (lang.code === 'en') continue
    it(`${lang.code} has no missing keys vs en`, () => {
      const paths = new Set(leafPaths(translations[lang.code]))
      const missing: string[] = []
      for (const p of EN_PATHS) {
        if (!paths.has(p)) missing.push(p)
      }
      expect(missing, `${lang.code} missing: ${missing.slice(0, 8).join(', ')}`).toEqual([])
    })
  }
})
