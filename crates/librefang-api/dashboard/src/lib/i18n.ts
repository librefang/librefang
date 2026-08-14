import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import en from "../locales/en.json";
import zh from "../locales/zh.json";
import uk from "../locales/uk.json";
import ko from "../locales/ko.json";
import pl from "../locales/pl.json";

export const normalizeDetectedLanguage = (language: string): string =>
  language.split(/[-_]/, 1)[0].toLowerCase();

export const i18nReady = i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      zh: { translation: zh },
      uk: { translation: uk },
      ko: { translation: ko },
      pl: { translation: pl },
    },
    load: "languageOnly",
    supportedLngs: ["en", "zh", "uk", "ko", "pl"],
    fallbackLng: "en",
    detection: {
      convertDetectedLanguage: normalizeDetectedLanguage,
    },
    interpolation: {
      // Keep i18next's default escaping (escapeValue: true). Translator-supplied
      // strings must never be rendered as live DOM — any HTML structure in a
      // translation goes through the <Trans> component with an explicit
      // `components` allowlist instead. See pages/MobilePairingPage.tsx and
      // pages/ConnectWizardPage.tsx for the pattern, and
      // docs/issues/i18n-escapeValue-false.md for the rationale.
      escapeValue: true,
    },
    ...(import.meta.env.DEV && {
      saveMissing: true,
      missingKeyHandler: (_lngs: readonly string[], ns: string, key: string) => {
        console.warn(`[i18n] missing key: ${ns}:${key}`);
      },
    }),
  })
  .catch((error: unknown) => {
    console.error("[i18n] initialization failed", error);
    throw error;
  });

export default i18n;
