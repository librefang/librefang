import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import LanguageDetector from "i18next-browser-languagedetector";

import en from "../locales/en.json";
import zh from "../locales/zh.json";

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources: {
      en: { translation: en },
      zh: { translation: zh },
    },
    fallbackLng: "en",
    interpolation: {
      // React already escapes interpolated values; double-escaping would break output
      escapeValue: false,
    },
    ...(import.meta.env.DEV && {
      saveMissing: true,
      missingKeyHandler: (_lngs: string[], ns: string, key: string) => {
        console.warn(`[i18n] missing key: ${ns}:${key}`);
      },
    }),
  });

export default i18n;
