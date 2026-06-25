import { languages, rawTranslations } from '../src/i18n'

function leafPaths(value: unknown, prefix = ''): string[] {
  if (Array.isArray(value)) {
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

function usage(): never {
  console.error('Usage: pnpm i18n:audit <locale|--all>')
  console.error(`Known locales: ${languages.map(lang => lang.code).join(', ')}`)
  process.exit(2)
}

const target = process.argv[2]
if (!target || process.argv.length > 3) usage()

const locales = target === '--all'
  ? languages.map(lang => lang.code).filter(code => code !== 'en')
  : [target]

const enPaths = new Set(leafPaths(rawTranslations.en))
let failed = false

for (const locale of locales) {
  const raw = rawTranslations[locale]
  if (!raw) {
    console.error(`Unknown locale: ${locale}`)
    failed = true
    continue
  }

  const paths = new Set(leafPaths(raw))
  const missing = [...enPaths].filter(path => !paths.has(path))
  if (missing.length === 0) {
    console.log(`${locale}: complete (${enPaths.size} keys)`)
    continue
  }

  failed = true
  console.error(`${locale}: missing ${missing.length} keys`)
  for (const path of missing) {
    console.error(`  - ${path}`)
  }
}

if (failed) process.exit(1)
