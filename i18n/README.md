# i18n — Internationalization Guide

This directory contains translated versions of the project README for different languages.

## Current Structure

```
i18n/
  README.de.md   # German (Deutsch)
  README.es.md   # Spanish (Espanol)
  README.ja.md   # Japanese (日本語)
  README.ko.md   # Korean (한국어)
  README.zh.md   # Chinese (中文)
```

Each file is a full translation of the root `README.md`. All translations follow the same structure and sections as the English original.

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
