//! Internationalization (i18n) module for API error messages.
//!
//! Provides a shared translation system using Project Fluent that can be used
//! across the LibreFang codebase (API server, CLI, etc.). Supports English,
//! Chinese, Spanish, Japanese, German, and French.
//!
//! # Usage
//!
//! ```rust,no_run
//! use librefang_types::i18n::ErrorTranslator;
//!
//! let translator = ErrorTranslator::new("zh-CN");
//! assert_eq!(translator.t("api-error-agent-not-found"), "Agent not found (in Chinese)");
//! ```

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

// Embed all locale files at compile time.
const EN_FTL: &str = include_str!("../locales/en/errors.ftl");
const ZH_CN_FTL: &str = include_str!("../locales/zh-CN/errors.ftl");
const ES_FTL: &str = include_str!("../locales/es/errors.ftl");
const JA_FTL: &str = include_str!("../locales/ja/errors.ftl");
const DE_FTL: &str = include_str!("../locales/de/errors.ftl");
const FR_FTL: &str = include_str!("../locales/fr/errors.ftl");

/// All languages supported by the error translation system.
pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "zh-CN", "es", "ja", "de", "fr"];

/// The default language used when no match is found.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Returns the Fluent source for a given language code.
fn ftl_source(lang: &str) -> &'static str {
    match lang {
        "zh-CN" => ZH_CN_FTL,
        "es" => ES_FTL,
        "ja" => JA_FTL,
        "de" => DE_FTL,
        "fr" => FR_FTL,
        _ => EN_FTL,
    }
}

/// Resolves a language code to the best matching supported language.
///
/// Tries exact match first, then prefix match (e.g., "zh" matches "zh-CN",
/// "en-US" matches "en"). Falls back to [`DEFAULT_LANGUAGE`].
pub fn resolve_language(requested: &str) -> &'static str {
    // Exact match
    for lang in SUPPORTED_LANGUAGES {
        if requested.eq_ignore_ascii_case(lang) {
            return lang;
        }
    }
    // Prefix match: "zh" -> "zh-CN", "en-US" -> "en"
    let prefix = requested.split('-').next().unwrap_or(requested);
    for lang in SUPPORTED_LANGUAGES {
        let lang_prefix = lang.split('-').next().unwrap_or(lang);
        if prefix.eq_ignore_ascii_case(lang_prefix) {
            return lang;
        }
    }
    DEFAULT_LANGUAGE
}

/// Parses an HTTP `Accept-Language` header and returns the best matching
/// supported language code.
///
/// Example input: `"zh-CN,zh;q=0.9,en;q=0.8"` returns `"zh-CN"`.
pub fn parse_accept_language(header: &str) -> &'static str {
    // Parse language tags with quality values, sort by quality descending.
    let mut candidates: Vec<(&str, f32)> = header
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let mut parts = entry.split(';');
            let lang = parts.next()?.trim();
            let quality = parts
                .find_map(|p| {
                    let p = p.trim();
                    p.strip_prefix("q=")
                        .and_then(|q| q.trim().parse::<f32>().ok())
                })
                .unwrap_or(1.0);
            Some((lang, quality))
        })
        .collect();

    // Sort by quality descending (stable sort preserves order for equal quality).
    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (lang, _) in &candidates {
        let resolved = resolve_language(lang);
        // Return if we found a non-default match, or if the user explicitly
        // requested the default language.
        if resolved != DEFAULT_LANGUAGE || lang.eq_ignore_ascii_case(DEFAULT_LANGUAGE) {
            return resolved;
        }
    }

    DEFAULT_LANGUAGE
}

/// A translator instance for a specific language.
///
/// Wraps a Fluent bundle and provides convenient methods for looking up
/// translated error messages by key, optionally with arguments.
pub struct ErrorTranslator {
    bundle: FluentBundle<FluentResource>,
    language: &'static str,
}

impl ErrorTranslator {
    /// Create a new translator for the given language code.
    ///
    /// If the language is not supported, falls back to English.
    pub fn new(language: &str) -> Self {
        let resolved = resolve_language(language);
        let lang_id: LanguageIdentifier = resolved
            .parse()
            .unwrap_or_else(|_| DEFAULT_LANGUAGE.parse().expect("en must parse"));
        let mut bundle = FluentBundle::new(vec![lang_id]);
        bundle.set_use_isolating(false);

        let source = ftl_source(resolved);
        let resource = FluentResource::try_new(source.to_string()).unwrap_or_else(|(_, _)| {
            // Fallback: if the resolved language pack is broken, use English.
            FluentResource::try_new(EN_FTL.to_string())
                .expect("English language pack must be valid")
        });

        if bundle.add_resource(resource).is_err() {
            // If adding the resource fails, create a fresh English bundle.
            let en_id: LanguageIdentifier = DEFAULT_LANGUAGE.parse().expect("en must parse");
            let mut en_bundle = FluentBundle::new(vec![en_id]);
            en_bundle.set_use_isolating(false);
            let en_resource = FluentResource::try_new(EN_FTL.to_string())
                .expect("English language pack must be valid");
            let _ = en_bundle.add_resource(en_resource);
            return Self {
                bundle: en_bundle,
                language: DEFAULT_LANGUAGE,
            };
        }

        Self {
            bundle,
            language: resolved,
        }
    }

    /// Look up a translation by key (no arguments).
    pub fn t(&self, key: &str) -> String {
        self.t_args(key, &[])
    }

    /// Look up a translation by key with named arguments.
    pub fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        let Some(message) = self.bundle.get_message(key) else {
            return key.to_string();
        };
        let Some(pattern) = message.value() else {
            return key.to_string();
        };

        let fluent_args = if args.is_empty() {
            None
        } else {
            let mut fa = FluentArgs::new();
            for (name, value) in args {
                fa.set(*name, FluentValue::from(*value));
            }
            Some(fa)
        };

        let mut errors = vec![];
        let result = self
            .bundle
            .format_pattern(pattern, fluent_args.as_ref(), &mut errors);
        result.to_string()
    }

    /// Returns the resolved language code for this translator.
    pub fn language(&self) -> &'static str {
        self.language
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_translation() {
        let t = ErrorTranslator::new("en");
        assert_eq!(t.t("api-error-agent-not-found"), "Agent not found");
    }

    #[test]
    fn chinese_translation() {
        let t = ErrorTranslator::new("zh-CN");
        assert_eq!(
            t.t("api-error-agent-not-found"),
            "\u{672a}\u{627e}\u{5230}\u{667a}\u{80fd}\u{4f53}"
        );
    }

    #[test]
    fn japanese_translation() {
        let t = ErrorTranslator::new("ja");
        // Contains katakana for "agent"
        assert!(t.t("api-error-agent-not-found").contains('\u{30A8}'));
    }

    #[test]
    fn spanish_translation() {
        let t = ErrorTranslator::new("es");
        assert_eq!(t.t("api-error-agent-not-found"), "Agente no encontrado");
    }

    #[test]
    fn german_translation() {
        let t = ErrorTranslator::new("de");
        assert_eq!(t.t("api-error-agent-not-found"), "Agent nicht gefunden");
    }

    #[test]
    fn french_translation() {
        let t = ErrorTranslator::new("fr");
        assert_eq!(t.t("api-error-agent-not-found"), "Agent non trouve");
    }

    #[test]
    fn translation_with_args() {
        let t = ErrorTranslator::new("en");
        let result = t.t_args("api-error-template-not-found", &[("name", "my-agent")]);
        // Fluent may produce ASCII or Unicode curly quotes depending on platform
        let normalized = result.replace(['\u{2018}', '\u{2019}'], "'");
        assert_eq!(normalized, "Template 'my-agent' not found");
    }

    #[test]
    fn chinese_translation_with_args() {
        let t = ErrorTranslator::new("zh-CN");
        let result = t.t_args("api-error-template-not-found", &[("name", "my-agent")]);
        assert!(result.contains("my-agent"));
    }

    #[test]
    fn unknown_key_returns_key() {
        let t = ErrorTranslator::new("en");
        assert_eq!(t.t("nonexistent-key"), "nonexistent-key");
    }

    #[test]
    fn fallback_to_english_for_unsupported_language() {
        let t = ErrorTranslator::new("ko");
        assert_eq!(t.language(), "en");
        assert_eq!(t.t("api-error-agent-not-found"), "Agent not found");
    }

    #[test]
    fn resolve_language_exact_match() {
        assert_eq!(resolve_language("zh-CN"), "zh-CN");
        assert_eq!(resolve_language("en"), "en");
        assert_eq!(resolve_language("ja"), "ja");
    }

    #[test]
    fn resolve_language_prefix_match() {
        assert_eq!(resolve_language("zh"), "zh-CN");
        assert_eq!(resolve_language("en-US"), "en");
        assert_eq!(resolve_language("es-MX"), "es");
        assert_eq!(resolve_language("ja-JP"), "ja");
        assert_eq!(resolve_language("de-AT"), "de");
        assert_eq!(resolve_language("fr-CA"), "fr");
    }

    #[test]
    fn resolve_language_case_insensitive() {
        assert_eq!(resolve_language("ZH-CN"), "zh-CN");
        assert_eq!(resolve_language("EN"), "en");
    }

    #[test]
    fn resolve_language_unsupported() {
        assert_eq!(resolve_language("ko"), "en");
        assert_eq!(resolve_language("ar"), "en");
    }

    #[test]
    fn parse_accept_language_simple() {
        assert_eq!(parse_accept_language("zh-CN"), "zh-CN");
        assert_eq!(parse_accept_language("en"), "en");
        assert_eq!(parse_accept_language("ja"), "ja");
    }

    #[test]
    fn parse_accept_language_with_quality() {
        assert_eq!(parse_accept_language("zh-CN,zh;q=0.9,en;q=0.8"), "zh-CN");
        assert_eq!(parse_accept_language("en;q=0.8,ja;q=0.9"), "ja");
    }

    #[test]
    fn parse_accept_language_unsupported_fallback() {
        assert_eq!(parse_accept_language("ko,ar"), "en");
    }

    #[test]
    fn parse_accept_language_empty() {
        assert_eq!(parse_accept_language(""), "en");
    }

    #[test]
    fn all_languages_have_same_keys() {
        let en = ErrorTranslator::new("en");
        let keys = [
            "api-error-agent-not-found",
            "api-error-agent-spawn-failed",
            "api-error-message-too-large",
            "api-error-auth-invalid-key",
            "api-error-auth-missing",
            "api-error-not-found",
            "api-error-internal",
        ];
        for lang in SUPPORTED_LANGUAGES {
            let t = ErrorTranslator::new(lang);
            for key in &keys {
                let result = t.t(key);
                assert_ne!(
                    result, *key,
                    "Language '{}' is missing translation for '{}'",
                    lang, key
                );
                if *lang != "en" {
                    let en_result = en.t(key);
                    assert_ne!(
                        result, en_result,
                        "Language '{}' has untranslated key '{}'",
                        lang, key
                    );
                }
            }
        }
    }
}
