//! Internationalization (i18n) module for the LibreFang CLI.

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use std::cell::RefCell;
use unic_langid::LanguageIdentifier;

const EN_FTL: &str = include_str!("../locales/en/main.ftl");
const ZH_CN_FTL: &str = include_str!("../locales/zh-CN/main.ftl");
const UK_FTL: &str = include_str!("../locales/uk/main.ftl");

pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "zh-CN", "uk"];
pub use librefang_types::i18n::DEFAULT_LANGUAGE;

thread_local! {
    static I18N: RefCell<Option<I18n>> = const { RefCell::new(None) };
}

pub struct I18n {
    bundle: FluentBundle<FluentResource>,
}

impl I18n {
    fn new(language: &str) -> Result<Self, String> {
        let selected = if SUPPORTED_LANGUAGES.contains(&language) {
            language
        } else {
            DEFAULT_LANGUAGE
        };
        let lang_id: LanguageIdentifier = selected
            .parse()
            .map_err(|e| format!("invalid language identifier: {e}"))?;
        let mut bundle = FluentBundle::new(vec![lang_id]);
        bundle.set_use_isolating(false);
        let source = match selected {
            "zh-CN" => ZH_CN_FTL,
            "uk" => UK_FTL,
            _ => EN_FTL,
        };
        let resource = FluentResource::try_new(source.to_string())
            .map_err(|(_, errors)| format!("failed to parse Fluent resource: {errors:?}"))?;
        bundle
            .add_resource(resource)
            .map_err(|errors| format!("failed to add Fluent resource: {errors:?}"))?;
        Ok(Self { bundle })
    }

    fn get(&self, key: &str, args: Option<&FluentArgs>) -> String {
        let Some(message) = self.bundle.get_message(key) else {
            return format!("[{key}]");
        };
        let Some(pattern) = message.value() else {
            return format!("[{key}]");
        };

        let mut errors = vec![];
        let result = self.bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!(key = %key, errors = ?errors, "Fluent formatting errors");
        }
        result.to_string()
    }
}

pub fn init(language: &str) {
    let i18n = I18n::new(language).unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to initialize i18n, falling back to English");
        I18n::new(DEFAULT_LANGUAGE).expect("default language pack must be valid")
    });
    I18N.with(|cell| {
        *cell.borrow_mut() = Some(i18n);
    });
}

fn is_utf8_locale() -> bool {
    let vars = ["LC_ALL", "LC_MESSAGES", "LANG"];
    for var in vars {
        if let Ok(val) = std::env::var(var) {
            let val_lower = val.to_lowercase();
            if val_lower.contains("utf8") || val_lower.contains("utf-8") {
                return true;
            }
            if val_lower == "c" || val_lower == "posix" {
                return false;
            }
            if let Some(dot_idx) = val.find('.') {
                let encoding = &val_lower[dot_idx + 1..];
                if encoding.contains("utf8") || encoding.contains("utf-8") {
                    return true;
                }
                return false;
            }
        }
    }
    true
}

pub fn detect_system_language() -> String {
    if !is_utf8_locale() {
        return DEFAULT_LANGUAGE.to_string();
    }
    let vars = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"];
    for var in vars {
        if let Ok(val) = std::env::var(var) {
            let parts = val.split([':', ';']);
            for part in parts {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let base = part.split(['.', '@']).next().unwrap_or(part);
                let normalized = base.replace('_', "-");

                // Try exact match
                for lang in SUPPORTED_LANGUAGES {
                    if lang.eq_ignore_ascii_case(&normalized) {
                        return lang.to_string();
                    }
                }

                // Try match by prefix before '-'
                let prefix = normalized.split('-').next().unwrap_or(&normalized);
                for lang in SUPPORTED_LANGUAGES {
                    if lang.eq_ignore_ascii_case(prefix) {
                        return lang.to_string();
                    }
                    let lang_prefix = lang.split('-').next().unwrap_or(lang);
                    if lang_prefix.eq_ignore_ascii_case(prefix) {
                        return lang.to_string();
                    }
                }
            }
        }
    }
    DEFAULT_LANGUAGE.to_string()
}

pub fn t(key: &str) -> String {
    I18N.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if borrow.is_none() {
            let i18n = I18n::new(DEFAULT_LANGUAGE).unwrap_or_else(|error| {
                panic!("failed to initialize default i18n fallback: {error}");
            });
            *borrow = Some(i18n);
        }
        borrow.as_ref().unwrap().get(key, None)
    })
}

pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    I18N.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if borrow.is_none() {
            let i18n = I18n::new(DEFAULT_LANGUAGE).unwrap_or_else(|error| {
                panic!("failed to initialize default i18n fallback: {error}");
            });
            *borrow = Some(i18n);
        }
        let i18n = borrow.as_ref().unwrap();
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, FluentValue::from(*value));
        }
        i18n.get(key, Some(&fluent_args))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_default_language() {
        init("fr");
        assert_eq!(t("daemon-starting"), "Starting daemon...");
    }

    #[test]
    fn renders_chinese_translation() {
        init("zh-CN");
        assert_eq!(t("label-dashboard"), "控制台");
    }

    #[test]
    fn renders_ukrainian_translation() {
        init("uk");
        assert_eq!(t("label-dashboard"), "Панель приладів");
    }

    #[test]
    fn renders_with_args() {
        init("en");
        assert_eq!(
            t_args("models-available", &[("count", "12")]),
            "12 models available"
        );
    }

    #[test]
    fn auto_initializes_when_not_initialized() {
        I18N.with(|cell| {
            *cell.borrow_mut() = None;
        });
        assert_eq!(t("daemon-starting"), "Starting daemon...");
    }

    #[test]
    fn test_detect_system_language() {
        let backup_language = std::env::var("LANGUAGE").ok();
        let backup_lang = std::env::var("LANG").ok();
        let backup_lc_all = std::env::var("LC_ALL").ok();
        let backup_lc_messages = std::env::var("LC_MESSAGES").ok();
        // macOS CI exports LC_ALL/LC_MESSAGES at UTF-8, which takes precedence over LANG and defeats the non-UTF-8 cases below.
        std::env::remove_var("LC_ALL");
        std::env::remove_var("LC_MESSAGES");
        std::env::remove_var("LANG");

        // Test matching "uk" from "uk:en_US"
        std::env::set_var("LANGUAGE", "uk:en_US");
        assert_eq!(detect_system_language(), "uk");

        // Test non-UTF-8 fallback to English even if LANGUAGE=uk:en_US is set
        std::env::set_var("LANG", "uk_UA.KOI8-U");
        assert_eq!(detect_system_language(), "en");

        // Test C locale fallback
        std::env::set_var("LANG", "C");
        assert_eq!(detect_system_language(), "en");

        // Test matching "zh-CN" from "zh_CN.UTF-8"
        std::env::remove_var("LANGUAGE");
        std::env::set_var("LANG", "zh_CN.UTF-8");
        assert_eq!(detect_system_language(), "zh-CN");

        // Test fallback to default
        std::env::set_var("LANG", "fr_FR.UTF-8");
        assert_eq!(detect_system_language(), "en");

        // Restore env vars
        if let Some(val) = backup_language {
            std::env::set_var("LANGUAGE", val);
        } else {
            std::env::remove_var("LANGUAGE");
        }
        if let Some(val) = backup_lang {
            std::env::set_var("LANG", val);
        } else {
            std::env::remove_var("LANG");
        }
        match backup_lc_all {
            Some(val) => std::env::set_var("LC_ALL", val),
            None => std::env::remove_var("LC_ALL"),
        }
        match backup_lc_messages {
            Some(val) => std::env::set_var("LC_MESSAGES", val),
            None => std::env::remove_var("LC_MESSAGES"),
        }
    }
}
