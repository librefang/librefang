import { useTranslation } from "react-i18next";
import { useUIStore } from "../lib/store";

export function SettingsPage() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage } = useUIStore();

  const sectionClass = "rounded-2xl border border-border-subtle bg-surface p-6 shadow-sm";
  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2 block";

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <header>
        <div className="flex items-center gap-2 text-brand font-bold uppercase tracking-widest text-[10px]">
          <svg className="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
          {t("settings.system_config")}
        </div>
        <h1 className="mt-2 text-3xl font-extrabold tracking-tight">{t("settings.title")}</h1>
        <p className="mt-1 text-text-dim font-medium">{t("settings.subtitle")}</p>
      </header>

      <div className="grid gap-6 lg:grid-cols-2">
        <section className={sectionClass}>
          <h2 className="text-lg font-black tracking-tight mb-6">{t("settings.appearance")}</h2>
          <div className="space-y-6">
            <div>
              <span className={labelClass}>{t("settings.theme")}</span>
              <div className="flex p-1 rounded-xl bg-main border border-border-subtle w-fit">
                <button onClick={() => theme !== 'light' && toggleTheme()} className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${theme === 'light' ? 'bg-surface text-brand shadow-sm' : 'text-text-dim'}`}>{t("settings.theme_light")}</button>
                <button onClick={() => theme !== 'dark' && toggleTheme()} className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${theme === 'dark' ? 'bg-surface text-brand shadow-sm' : 'text-text-dim'}`}>{t("settings.theme_dark")}</button>
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.language")}</span>
              <div className="flex p-1 rounded-xl bg-main border border-border-subtle w-fit">
                <button onClick={() => setLanguage('en')} className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${language === 'en' ? 'bg-surface text-brand shadow-sm' : 'text-text-dim'}`}>English</button>
                <button onClick={() => setLanguage('zh')} className={`px-4 py-2 rounded-lg text-xs font-bold transition-all ${language === 'zh' ? 'bg-surface text-brand shadow-sm' : 'text-text-dim'}`}>中文</button>
              </div>
            </div>
          </div>
        </section>
      </div>
    </div>
  );
}
