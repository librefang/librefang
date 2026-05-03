# i18n — Internationalization Guide

This directory contains translated versions of the project README and a small number of legacy translated long-form guides.

## Content surfaces and ownership

LibreFang has two surfaces for non-English content. **Pick the right one before you start a translation.**

| Content type | Source of truth | Translated copies live in |
|---|---|---|
| Project README (root `README.md`) | English at repo root | `i18n/README.<lang>.md` |
| Long-form guides (getting started, tutorials, deep dives, architecture) | Next.js docs site under `docs/src/app/` | Same Next.js route tree under `docs/src/app/<locale>/...` |
| Legacy long-form sidecars (e.g. `getting-started.fr.md`, `skill-development.zh.md`) | Historic | Already in `i18n/` — being migrated into the Next.js route tree |

**Rules of thumb:**

- New translated **README** content → drop a file at `i18n/README.<lang>.md` and update the language switcher in every existing README (root + every `i18n/README.*.md`). That switcher is the only way readers discover translations.
- New translated **long-form guide** → add it under `docs/src/app/<locale>/<slug>/` in the Next.js docs site, **not** as a sidecar `.<lang>.md` in `i18n/`. The `i18n/` sidecar layout is legacy and being phased out.
- Existing sidecars in `i18n/` (e.g. `getting-started.fr.md`, `skill-development.zh.md`) remain reachable via links from the corresponding `i18n/README.<lang>.md` until they are migrated into the docs site.

## Current Structure

```
i18n/
  README.de.md   # German (Deutsch)
  README.es.md   # Spanish (Español)
  README.fr.md   # French (Français)
  README.ja.md   # Japanese (日本語)
  README.ko.md   # Korean (한국어)
  README.pl.md   # Polish (Polski)
  README.zh.md   # Chinese (中文)

  # Legacy long-form sidecars — slated to migrate into docs/src/app/<locale>/...
  getting-started.fr.md   # linked from README.fr.md
  skill-development.zh.md # linked from README.zh.md
```

Each `README.<lang>.md` is a translation of the root `README.md`. All translations follow the same structure and sections as the English original.

## How to Add a New Language

1. Copy the English `README.md` from the project root into this directory, naming it `README.<lang>.md` where `<lang>` is the [ISO 639-1 language code](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes) (e.g., `fr` for French, `pt` for Portuguese).

2. Translate all content into the target language.

3. Update the multi-language navigation bar in your new file and in **all existing translation files** (including the root `README.md`). The navigation bar looks like this:
   ```html
   <strong>Multi-language:</strong>
   <a href="../README.md">English</a> | <a href="README.zh.md">中文</a> | ...
   ```
   Add a link for your new language to this bar in every file.

4. Keep all relative links (e.g., `../CONTRIBUTING.md`, `../GOVERNANCE.md`) pointing to the English originals in the repo root -- do not duplicate those files.

5. Submit a PR with your changes.

## How to Add New Translation Keys

This project uses full-document translations rather than key-value translation files. When the English `README.md` is updated with new sections or content:

1. Check what changed in the root `README.md` (review the diff).
2. Add the corresponding translated sections to each language file in the same position.
3. If you cannot translate to all languages, update the ones you can and open issues for the remaining languages so other contributors can help.

## Style Guidelines

- **Keep translations concise.** Match the tone and length of the English original. Avoid adding extra commentary.
- **Preserve all placeholders and markup.** HTML tags, badge URLs, image paths, and link targets must remain unchanged.
- **Preserve formatting.** Keep the same heading levels, table structure, and code blocks as the original.
- **Use natural phrasing.** Prefer idiomatic expressions in the target language over literal word-for-word translation.
- **Technical terms.** Keep well-known technical terms (e.g., "Rust", "crate", "CLI", "API", "WebAssembly") in English. Translate descriptive terms around them.
- **Consistent terminology.** Use the same translated term for a concept throughout the entire document. For example, if you translate "agent" as a specific word in your language, use that word everywhere.
- **Brand names stay in English.** "LibreFang", "Hands", "Hand" (as product names), "FangHub", and other proper nouns should not be translated.

## How to Test Translations

1. **Visual review:** Open the markdown file in a GitHub preview or any markdown viewer to verify formatting renders correctly.
2. **Link check:** Verify all relative links (`../README.md`, `../CONTRIBUTING.md`, etc.) resolve correctly from the `i18n/` directory.
3. **Badge check:** Ensure shield.io badges and image URLs display properly.
4. **Navigation check:** Click through the multi-language navigation bar to confirm all language links work.
5. **Diff comparison:** Compare your translation against the English original section by section to ensure nothing is missing.

## Related Documentation

- [CONTRIBUTING.md](../CONTRIBUTING.md) — General contribution guidelines
- [GOVERNANCE.md](../GOVERNANCE.md) — Project governance
- [README.md](../README.md) — English original
