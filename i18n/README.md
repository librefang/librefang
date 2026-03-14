# LibreFang Internationalization (i18n)

This directory contains translated versions of the project README.

## Current Translations

| Language | File | Status |
|----------|------|--------|
| English | [README.md](../README.md) | Source |
| Chinese (中文) | [README.zh.md](README.zh.md) | Complete |
| Japanese (日本語) | [README.ja.md](README.ja.md) | Complete |
| Korean (한국어) | [README.ko.md](README.ko.md) | Complete |
| Spanish (Español) | [README.es.md](README.es.md) | Complete |
| German (Deutsch) | [README.de.md](README.de.md) | Complete |

## Translation File Structure

Each translation is a complete copy of the main `README.md`, translated into the target language. Files follow the naming convention `README.<lang-code>.md` where `<lang-code>` is an [ISO 639-1](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes) two-letter language code.

All translated READMEs live in this `i18n/` directory. The English source `README.md` stays at the repository root.

## How to Add a New Language

1. **Copy the English README** as your starting point:

   ```bash
   cp README.md i18n/README.<lang-code>.md
   ```

2. **Translate the content**. Keep the same Markdown structure (headings, tables, code blocks). Do NOT translate:
   - Code snippets and command-line examples
   - File paths and crate names
   - API endpoint paths
   - Brand names (LibreFang, Rust, GitHub, etc.)

3. **Update the language selector** at the top of your translated file to include all languages, with yours marked as current.

4. **Update this README** by adding your language to the table above.

5. **Add the language link** to the language selector in every other translated README and the root `README.md`.

6. **Submit a PR** with the title `docs: add <Language> translation`.

## Style Guidelines

- **Tone**: Match the original — friendly, direct, and technical.
- **Terminology**: Use widely accepted technical terms in your language. If a term has no standard translation (e.g., "agent", "kernel"), keep the English term.
- **Formatting**: Preserve all Markdown formatting, badges, and links. Only translate human-readable text.
- **Consistency**: If a term appears multiple times, translate it the same way throughout.
- **Updates**: When the English README changes, translations may need updating. Check `git log -- README.md` to see what changed.
