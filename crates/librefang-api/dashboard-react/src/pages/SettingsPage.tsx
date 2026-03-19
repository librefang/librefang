import { useTranslation } from "react-i18next";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Globe, Sun, Moon, Settings, PanelLeftClose, PanelLeft } from "lucide-react";
import { useUIStore } from "../lib/store";

export function SettingsPage() {
  const { t } = useTranslation();
  const { theme, toggleTheme, language, setLanguage, navLayout, setNavLayout } = useUIStore();

  const labelClass = "text-[10px] font-black uppercase tracking-widest text-text-dim mb-2 block";

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
        <Card padding="lg" hover>
          <h2 className="text-lg font-black tracking-tight mb-6">{t("settings.appearance")}</h2>
          <div className="space-y-8">
            <div>
              <span className={labelClass}>{t("settings.theme")}</span>
              <div className="flex gap-3">
                <Button
                  variant={theme === 'light' ? "primary" : "secondary"}
                  onClick={() => theme !== 'light' && toggleTheme()}
                >
                  <Sun className="h-4 w-4" />
                  {t("settings.theme_light")}
                </Button>
                <Button
                  variant={theme === 'dark' ? "primary" : "secondary"}
                  onClick={() => theme !== 'dark' && toggleTheme()}
                >
                  <Moon className="h-4 w-4" />
                  {t("settings.theme_dark")}
                </Button>
              </div>
            </div>
            <div>
              <span className={labelClass}>{t("settings.language")}</span>
              <div className="flex gap-3">
                <Button
                  variant={language === 'en' ? "primary" : "secondary"}
                  onClick={() => setLanguage('en')}
                >
                  <Globe className="h-4 w-4" />
                  English
                </Button>
                <Button
                  variant={language === 'zh' ? "primary" : "secondary"}
                  onClick={() => setLanguage('zh')}
                >
                  <Globe className="h-4 w-4" />
                  中文
                </Button>
              </div>
            </div>
            <div>
              <span className={labelClass}>导航菜单布局</span>
              <div className="flex gap-3">
                <Button
                  variant={navLayout === "grouped" ? "primary" : "secondary"}
                  onClick={() => setNavLayout("grouped")}
                >
                  <PanelLeft className="h-4 w-4" />
                  分组
                </Button>
                <Button
                  variant={navLayout === "collapsible" ? "primary" : "secondary"}
                  onClick={() => setNavLayout("collapsible")}
                >
                  <PanelLeftClose className="h-4 w-4" />
                  二级菜单
                </Button>
              </div>
            </div>
          </div>
        </Card>
      </div>
    </div>
  );
}
