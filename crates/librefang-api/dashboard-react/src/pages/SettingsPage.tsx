import { useTranslation } from "react-i18next";
import { Globe, Sun, Moon, Settings } from "lucide-react";
import { useUIStore } from "../lib/store";

export function SettingsPage() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage } = useUIStore();

  const sectionClass = "rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm hover:border-brand/30 transition-all";
  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2 block";
  const activeBtnClass = "px-5 py-2.5 rounded-xl text-sm font-bold transition-all bg-brand text-white shadow-lg shadow-brand/20";
  const inactiveBtnClass = "px-5 py-2.5 rounded-xl text-sm font-bold transition-all text-text-dim hover:text-brand hover:bg-surface-hover";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header>
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <Settings className="h-4 w-4" />
          {t("settings.system_config")}
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("settings.title")}</h1>
        <p className="mt-1 text-text-dim font-medium">{t("settings.subtitle")}</p>
      </header>

      <div className="grid gap-6 lg:grid-cols-2">
        <section className={sectionClass}>
          <h2 className="text-lg font-black tracking-tight mb-6">{t("settings.appearance")}</h2>
          <div className="space-y-8">
            <div>
              <span className={labelClass}>{t("settings.theme")}</span>
              <div className="flex gap-3">
                <button
                  onClick={() => theme !== 'light' && toggleTheme()}
                  className={theme === 'light' ? activeBtnClass : inactiveBtnClass}
                >
                  <span className="flex items-center gap-2">
                    <Sun className="h-4 w-4" />
                    {t("settings.theme_light")}
                  </span>
                </button>
                <button
                  onClick={() => theme !== 'dark' && toggleTheme()}
                  className={theme === 'dark' ? activeBtnClass : inactiveBtnClass}
                >
                  <span className="flex items-center gap-2">
                    <Moon className="h-4 w-4" />
                    {t("settings.theme_dark")}
                  </span>
                </button>
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.language")}</span>
              <div className="flex gap-3">
                <button
                  onClick={() => setLanguage('en')}
                  className={language === 'en' ? activeBtnClass : inactiveBtnClass}
                >
                  <span className="flex items-center gap-2">
                    <Globe className="h-4 w-4" />
                    English
                  </span>
                </button>
                <button
                  onClick={() => setLanguage('zh')}
                  className={language === 'zh' ? activeBtnClass : inactiveBtnClass}
                >
                  <span className="flex items-center gap-2">
                    <Globe className="h-4 w-4" />
                    中文
                  </span>
                </button>
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
