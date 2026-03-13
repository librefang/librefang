//! Internationalization (i18n) module for the LibreFang CLI.

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use std::cell::RefCell;
use unic_langid::LanguageIdentifier;

const EN_FTL: &str = include_str!("../locales/en/main.ftl");
const ZH_CN_FTL: &str = include_str!("../locales/zh-CN/main.ftl");

pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "zh-CN"];
pub const DEFAULT_LANGUAGE: &str = "en";

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

pub fn t(key: &str) -> String {
    I18N.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|i18n| i18n.get(key, None))
            .unwrap_or_else(|| format!("[{key}]"))
    })
}

pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    I18N.with(|cell| {
        if let Some(i18n) = cell.borrow().as_ref() {
            let mut fluent_args = FluentArgs::new();
            for (name, value) in args {
                fluent_args.set(*name, FluentValue::from(*value));
            }
            i18n.get(key, Some(&fluent_args))
        } else {
            format!("[{key}]")
        }
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
    fn renders_with_args() {
        init("en");
        assert_eq!(
            t_args("models-available", &[("count", "12")]),
            "12 models available"
        );
    }
}
