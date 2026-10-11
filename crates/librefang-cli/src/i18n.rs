//! Internationalization (i18n) module for the LibreFang CLI.

use fluent::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use std::cell::RefCell;
use unic_langid::LanguageIdentifier;

const EN_FTL: &str = include_str!("../locales/en/main.ftl");
const ZH_CN_FTL: &str = include_str!("../locales/zh-CN/main.ftl");
const UK_FTL: &str = include_str!("../locales/uk/main.ftl");
const KO_FTL: &str = include_str!("../locales/ko/main.ftl");

pub const SUPPORTED_LANGUAGES: &[&str] = &["en", "zh-CN", "uk", "ko"];
pub use librefang_types::i18n::DEFAULT_LANGUAGE;

/// Maps a supported language to its embedded `.ftl` source.
///
/// This is the single place production code and tests both go through to
/// resolve a language to its Fluent source, so a new entry in
/// `SUPPORTED_LANGUAGES` cannot silently fall out of sync with the locale
/// files the duplicate-key guard test below scans.
fn locale_source(language: &str) -> &'static str {
    match language {
        "zh-CN" => ZH_CN_FTL,
        "uk" => UK_FTL,
        "ko" => KO_FTL,
        _ => EN_FTL,
    }
}

thread_local! {
    static I18N: RefCell<Option<I18n>> = const { RefCell::new(None) };
}

/// The language the process selected at startup, recorded by [`init_process`].
///
/// The per-thread slot above is filled lazily on a thread's first lookup, and every TUI fetch/save runs on a freshly spawned thread.
/// Without this, those threads fell back to `DEFAULT_LANGUAGE` and printed their error and status lines in English next to translated labels.
static PROCESS_LANGUAGE: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The `SUPPORTED_LANGUAGES` entry matching `language`, or `DEFAULT_LANGUAGE` when there is none.
fn resolve_supported(language: &str) -> &'static str {
    SUPPORTED_LANGUAGES
        .iter()
        .copied()
        .find(|l| *l == language)
        .unwrap_or(DEFAULT_LANGUAGE)
}

/// The language a thread that never called [`init`] starts with.
fn thread_default_language() -> &'static str {
    PROCESS_LANGUAGE.get().copied().unwrap_or(DEFAULT_LANGUAGE)
}

pub struct I18n {
    bundle: FluentBundle<FluentResource>,
    fallback_bundle: FluentBundle<FluentResource>,
}

impl I18n {
    fn new(language: &str) -> Result<Self, String> {
        let selected = if SUPPORTED_LANGUAGES.contains(&language) {
            language
        } else {
            DEFAULT_LANGUAGE
        };
        let bundle = Self::bundle_for(selected)?;
        let fallback_bundle = Self::bundle_for(DEFAULT_LANGUAGE)?;
        Ok(Self {
            bundle,
            fallback_bundle,
        })
    }

    fn bundle_for(language: &str) -> Result<FluentBundle<FluentResource>, String> {
        let lang_id: LanguageIdentifier = language
            .parse()
            .map_err(|e| format!("invalid language identifier: {e}"))?;
        let mut bundle = FluentBundle::new(vec![lang_id]);
        bundle.set_use_isolating(false);
        let source = locale_source(language);
        let resource = FluentResource::try_new(source.to_string())
            .map_err(|(_, errors)| format!("failed to parse Fluent resource: {errors:?}"))?;
        bundle
            .add_resource(resource)
            .map_err(|errors| format!("failed to add Fluent resource: {errors:?}"))?;
        Ok(bundle)
    }

    fn get(&self, key: &str, args: Option<&FluentArgs>) -> String {
        Self::get_from_bundle(&self.bundle, key, args)
            .or_else(|| Self::get_from_bundle(&self.fallback_bundle, key, args))
            .unwrap_or_else(|| format!("[{key}]"))
    }

    fn get_from_bundle(
        bundle: &FluentBundle<FluentResource>,
        key: &str,
        args: Option<&FluentArgs>,
    ) -> Option<String> {
        let message = bundle.get_message(key)?;
        let pattern = message.value()?;

        let mut errors = vec![];
        let result = bundle.format_pattern(pattern, args, &mut errors);
        if !errors.is_empty() {
            tracing::warn!(key = %key, errors = ?errors, "Fluent formatting errors");
        }
        Some(result.to_string())
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

/// Select the language for the whole process: the calling thread, and every thread whose first lookup happens later.
///
/// Only `main` calls this.
/// [`init`] stays thread-local so unit tests, which each run on their own thread, can pick a language without changing it for their neighbours.
pub fn init_process(language: &str) {
    let _ = PROCESS_LANGUAGE.set(resolve_supported(language));
    init(language);
}

/// Run `f` against this thread's bundle, building it in the process language on first use.
fn with_i18n<R>(f: impl FnOnce(&I18n) -> R) -> R {
    I18N.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let i18n = borrow.get_or_insert_with(|| {
            I18n::new(thread_default_language()).unwrap_or_else(|error| {
                panic!("failed to initialize default i18n fallback: {error}");
            })
        });
        f(i18n)
    })
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
    with_i18n(|i18n| i18n.get(key, None))
}

pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    with_i18n(|i18n| {
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

    /// Returns `(message_id, line_number)` for every top-level message/term
    /// definition in a `.ftl` source, in encounter order (duplicates included).
    ///
    /// A definition is a line starting in column 0 with a valid Fluent
    /// identifier (`[A-Za-z][A-Za-z0-9_-]*`, optionally prefixed with `-` for
    /// terms) followed by `=`. Everything else at column 0 is a comment
    /// (`#`); everything indented is either an attribute (`.attr = ...`) or a
    /// continuation line of a multiline value, neither of which introduces a
    /// new top-level key.
    fn find_message_definitions(source: &str) -> Vec<(String, usize)> {
        fn is_identifier(s: &str) -> bool {
            let mut chars = s.chars();
            matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        }

        let mut defs = Vec::new();
        for (idx, line) in source.lines().enumerate() {
            let Some(first_char) = line.chars().next() else {
                continue; // blank line
            };
            if first_char.is_whitespace() || first_char == '#' {
                continue; // continuation, attribute, or comment
            }
            let Some(eq_idx) = line.find('=') else {
                continue; // not a definition line (e.g. a `[[ ... ]]` selector edge case)
            };
            let candidate = line[..eq_idx].trim();
            let is_definition = candidate
                .strip_prefix('-')
                .map(is_identifier)
                .unwrap_or_else(|| is_identifier(candidate));
            if is_definition {
                defs.push((candidate.to_string(), idx + 1));
            }
        }
        defs
    }

    /// Guards against the failure mode from #8291-style merges: `union` is a
    /// text-concatenating git merge driver, so two branches that each add a
    /// key to the same `.ftl` file merge cleanly and leave both copies
    /// behind, with no merge conflict to flag it.
    ///
    /// Fluent then rejects the *entire* resource for that language rather
    /// than keeping one copy: `FluentBundle::add_resource` reports every
    /// duplicate id as an `Overriding` error, which makes `bundle_for(..)`
    /// return `Err`. When the duplicate lands in the default-language pack,
    /// `I18n::new(DEFAULT_LANGUAGE).expect(..)` in `init()` panics — this is
    /// exactly what happened when `tui-agents-hints-detail` ended up defined
    /// three times in `en/main.ftl`: `i18n::init` panicked on the default
    /// pack and took 138 unrelated `librefang-cli` tests down with it (doctor,
    /// purge, desktop_install, every TUI screen, all eleven
    /// `daemon_failure_exit_codes` cases), with a panic message that names
    /// Fluent, not the merge that caused it.
    ///
    /// When the duplicate lands in a non-default pack instead, there is no
    /// panic: `bundle_for` fails for that language only, silently falling
    /// back to English for every key. That is what happened to 30
    /// model-routing keys duplicated only in `zh-CN/main.ftl` (2512
    /// definitions there against 2482 in `en`, `ko`, and `uk`) — the whole
    /// Chinese pack silently stopped loading, and nothing failed except one
    /// hand-written per-language assertion (`renders_chinese_translation`).
    #[test]
    fn locale_files_have_no_duplicate_message_keys() {
        let mut failures = Vec::new();
        for &language in SUPPORTED_LANGUAGES {
            let path = format!("locales/{language}/main.ftl");
            let mut lines_by_key: std::collections::BTreeMap<&str, Vec<usize>> =
                std::collections::BTreeMap::new();
            let defs = find_message_definitions(locale_source(language));
            for (key, line_no) in &defs {
                lines_by_key.entry(key.as_str()).or_default().push(*line_no);
            }
            for (key, lines) in lines_by_key {
                if lines.len() > 1 {
                    failures.push(format!(
                        "{path}: `{key}` is defined {} times, at lines {lines:?}",
                        lines.len()
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "duplicate Fluent message keys found (each one breaks Fluent's parsing \
             for that entire locale pack — see the doc comment on this test):\n  {}",
            failures.join("\n  ")
        );
    }

    /// All four packs are hand-maintained translations of the same message
    /// set. As of this writing they define exactly the same 2398 keys; this
    /// pins that invariant so a key added to one locale and silently missed
    /// in another (leaving `[key-name]` or an English fallback rendered)
    /// gets caught here instead of by a user noticing untranslated text.
    #[test]
    fn locale_files_define_the_same_key_set() {
        let en_keys: std::collections::BTreeSet<String> = find_message_definitions(EN_FTL)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        for &language in SUPPORTED_LANGUAGES {
            if language == DEFAULT_LANGUAGE {
                continue;
            }
            let path = format!("locales/{language}/main.ftl");
            let keys: std::collections::BTreeSet<String> =
                find_message_definitions(locale_source(language))
                    .into_iter()
                    .map(|(key, _)| key)
                    .collect();
            let missing: Vec<&String> = en_keys.difference(&keys).collect();
            let extra: Vec<&String> = keys.difference(&en_keys).collect();
            assert!(
                missing.is_empty() && extra.is_empty(),
                "{path} key set diverges from {}/main.ftl: missing {missing:?}, extra {extra:?}",
                DEFAULT_LANGUAGE
            );
        }
    }

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
    fn renders_korean_translation() {
        init("ko");
        assert_eq!(t("label-dashboard"), "대시보드");
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
    fn goals_labels_preserve_significant_whitespace() {
        init("en");
        assert_eq!(t("tui-goals-judge-label"), "  Goal Judge: ");
        assert_eq!(t("tui-goals-phase-label"), "  Run Phase: ");
    }

    #[test]
    fn falls_back_to_english_for_missing_locale_key() {
        let lang_id: LanguageIdentifier = "zh-CN".parse().expect("language id must parse");
        let mut localized_bundle = FluentBundle::new(vec![lang_id]);
        localized_bundle.set_use_isolating(false);
        let resource =
            FluentResource::try_new("label-dashboard = 本地化控制台".to_string()).unwrap();
        localized_bundle.add_resource(resource).unwrap();
        let fallback_bundle = I18n::bundle_for("en").expect("English bundle must be valid");
        let i18n = I18n {
            bundle: localized_bundle,
            fallback_bundle,
        };

        assert_eq!(
            i18n.get(
                "tui-init-migrate-openclaw-skills",
                Some(&{
                    let mut args = FluentArgs::new();
                    args.set("count", FluentValue::from("3"));
                    args
                }),
            ),
            "3 skills"
        );
        assert_eq!(i18n.get("missing-test-key", None), "[missing-test-key]");
    }

    /// Env var that marks the child run of [`spawned_thread_inherits_process_language`].
    const INHERIT_CHILD_ENV: &str = "LIBREFANG_I18N_INHERIT_CHILD";

    /// A thread spawned after `init_process` must render in the process language, not English.
    ///
    /// `PROCESS_LANGUAGE` is a write-once process global, and libtest runs every test on its own fresh thread, so setting it in this binary would switch every English-asserting test that starts afterwards to Chinese.
    /// The assertion therefore runs in a child copy of this test binary, filtered to this one test, where nothing else can observe the global.
    #[test]
    fn spawned_thread_inherits_process_language() {
        if std::env::var_os(INHERIT_CHILD_ENV).is_some() {
            init_process("zh-CN");
            let got = std::thread::spawn(|| t("label-dashboard"))
                .join()
                .expect("worker thread");
            assert_eq!(got, "控制台");
            let got = std::thread::spawn(|| t_args("models-available", &[("count", "12")]))
                .join()
                .expect("worker thread");
            assert_ne!(got, "12 models available", "t_args must also inherit");
            return;
        }
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args([
                "i18n::tests::spawned_thread_inherits_process_language",
                "--exact",
                "--test-threads=1",
            ])
            .env(INHERIT_CHILD_ENV, "1")
            .output()
            .expect("run child test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "child run failed\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("1 passed"),
            "the child must actually run the test: {stdout}"
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
