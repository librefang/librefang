import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";

type ScheduleType = "interval_min" | "interval_hour" | "daily" | "weekday" | "weekly" | "monthly" | "custom";

interface ScheduleModalProps {
  title: string;
  subtitle?: string;
  initialCron?: string;
  initialTz?: string;
  onSave: (cron: string, tz?: string) => void;
  onClose: () => void;
}

/** Common IANA timezones for the picker. */
const COMMON_TIMEZONES = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Sao_Paulo",
  "Europe/London",
  "Europe/Berlin",
  "Europe/Paris",
  "Europe/Rome",
  "Europe/Moscow",
  "Asia/Dubai",
  "Asia/Kolkata",
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Australia/Sydney",
  "Pacific/Auckland",
];

const HOURS = Array.from({ length: 24 }, (_, i) => i);
const MINUTES = [0, 5, 10, 15, 20, 30, 45];

function parseCronType(cron: string): { type: ScheduleType; min?: number; hour?: number; day?: number; weekday?: number; interval?: number } {
  const parts = cron.split(/\s+/);
  if (parts.length !== 5) return { type: "custom" };
  const [m, h, dom, , dow] = parts;
  if (m.startsWith("*/") && h === "*") return { type: "interval_min", interval: parseInt(m.slice(2)) || 5 };
  if (m === "0" && h.startsWith("*/")) return { type: "interval_hour", interval: parseInt(h.slice(2)) || 1 };
  if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom.match(/^\d+$/) && dow === "*") return { type: "monthly", hour: +h, min: +m, day: +dom };
  if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow.match(/^\d$/)) return { type: "weekly", hour: +h, min: +m, weekday: +dow };
  if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow === "1-5") return { type: "weekday", hour: +h, min: +m };
  if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow === "*") return { type: "daily", hour: +h, min: +m };
  return { type: "custom" };
}

/** Try to detect the browser's IANA timezone. */
function detectBrowserTimezone(): string {
  try {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    return tz || "UTC";
  } catch {
    return "UTC";
  }
}

export function ScheduleModal({ title, subtitle, initialCron, initialTz, onSave, onClose }: ScheduleModalProps) {
  const { t, i18n } = useTranslation();
  const isZh = i18n.language?.startsWith("zh");

  const parsed = parseCronType(initialCron || "0 9 * * *");
  const detectedTz = initialTz || detectBrowserTimezone();
  const [timezone, setTimezone] = useState(detectedTz);

  // If the detected/initial timezone isn't in the common list, include it
  const timezoneOptions = COMMON_TIMEZONES.includes(detectedTz)
    ? COMMON_TIMEZONES
    : [detectedTz, ...COMMON_TIMEZONES];
  const [scheduleType, setScheduleType] = useState<ScheduleType>(parsed.type);
  const [intervalMin, setIntervalMin] = useState(parsed.type === "interval_min" ? (parsed.interval ?? 5) : 5);
  const [intervalHour, setIntervalHour] = useState(parsed.type === "interval_hour" ? (parsed.interval ?? 1) : 1);
  const [hour, setHour] = useState(parsed.hour ?? 9);
  const [minute, setMinute] = useState(parsed.min ?? 0);
  const [weekday, setWeekday] = useState(parsed.weekday ?? 1);
  const [monthDay, setMonthDay] = useState(parsed.day ?? 1);
  const [customCron, setCustomCron] = useState(initialCron || "0 9 * * *");

  const buildCron = (): string => {
    switch (scheduleType) {
      case "interval_min": return `*/${intervalMin} * * * *`;
      case "interval_hour": return `0 */${intervalHour} * * *`;
      case "daily": return `${minute} ${hour} * * *`;
      case "weekday": return `${minute} ${hour} * * 1-5`;
      case "weekly": return `${minute} ${hour} * * ${weekday}`;
      case "monthly": return `${minute} ${hour} ${monthDay} * *`;
      case "custom": return customCron;
    }
  };

  const validateCron = (cron: string): boolean => {
    const parts = cron.trim().split(/\s+/);
    if (parts.length !== 5) return false;
    return parts.every(p => /^(\*|(\*\/)?[0-9]+([-,/][0-9]+)*)$/.test(p));
  };

  const describeCron = (cron: string): string => {
    const parts = cron.trim().split(/\s+/);
    if (parts.length !== 5) return t("scheduler.cron_invalid");
    const [m, h, dom, , dow] = parts;
    const pad = (n: string) => n.padStart(2, "0");
    const weekdays = [
      t("scheduler.weekday_sun"), t("scheduler.weekday_mon"), t("scheduler.weekday_tue"),
      t("scheduler.weekday_wed"), t("scheduler.weekday_thu"), t("scheduler.weekday_fri"),
      t("scheduler.weekday_sat"),
    ];
    const time = `${pad(h)}:${pad(m)}`;
    if (m.startsWith("*/") && h === "*") return t("scheduler.cron_every_n_min", { n: m.slice(2) });
    if (m === "0" && h.startsWith("*/")) return t("scheduler.cron_every_n_hour", { n: h.slice(2) });
    if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom.match(/^\d+$/) && dow === "*") return t("scheduler.cron_monthly", { dom, time });
    if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow.match(/^\d$/)) return t("scheduler.cron_weekly", { day: weekdays[+dow], time });
    if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow === "1-5") return t("scheduler.cron_weekdays", { time });
    if (m.match(/^\d+$/) && h.match(/^\d+$/) && dom === "*" && dow === "*") return t("scheduler.cron_daily", { time });
    return cron;
  };

  const [previewCron, setPreviewCron] = useState(buildCron());
  useEffect(() => setPreviewCron(buildCron()), [scheduleType, intervalMin, intervalHour, hour, minute, weekday, monthDay, customCron]);
  const cronValid = validateCron(previewCron);

  const types: { key: ScheduleType; label: string }[] = [
    { key: "interval_min", label: t("scheduler.type_interval_min") },
    { key: "interval_hour", label: t("scheduler.type_interval_hour") },
    { key: "daily", label: t("scheduler.type_daily") },
    { key: "weekday", label: t("scheduler.type_weekday") },
    { key: "weekly", label: t("scheduler.type_weekly") },
    { key: "monthly", label: t("scheduler.type_monthly") },
    { key: "custom", label: t("scheduler.type_custom") },
  ];

  const sel = "h-9 rounded-lg border border-border-subtle bg-main px-2 text-sm outline-none focus:border-brand transition-colors";
  const num = "h-9 w-16 rounded-lg border border-border-subtle bg-main px-2 text-sm font-mono text-center outline-none focus:border-brand transition-colors";

  const timeSelect = (
    <div className="flex items-center gap-0.5">
      <select value={hour} onChange={e => setHour(+e.target.value)} className={sel}>
        {HOURS.map(h => <option key={h} value={h}>{String(h).padStart(2, "0")}</option>)}
      </select>
      <span className="text-text-dim font-bold">:</span>
      <select value={minute} onChange={e => setMinute(+e.target.value)} className={sel}>
        {MINUTES.map(m => <option key={m} value={m}>{String(m).padStart(2, "0")}</option>)}
      </select>
    </div>
  );

  const wdShort = [
    t("scheduler.weekday_short_mon"), t("scheduler.weekday_short_tue"),
    t("scheduler.weekday_short_wed"), t("scheduler.weekday_short_thu"),
    t("scheduler.weekday_short_fri"), t("scheduler.weekday_short_sat"),
    t("scheduler.weekday_short_sun"),
  ];

  const cronFieldHeaders = [
    t("scheduler.field_min"), t("scheduler.field_hour"),
    t("scheduler.field_day"), t("scheduler.field_month"),
    t("scheduler.field_weekday"),
  ];

  return (
    <div className="fixed inset-0 z-100 flex items-center justify-center bg-black/50 backdrop-blur-sm" onClick={onClose}>
      <div className="w-full max-w-140 mx-4 rounded-2xl bg-surface border border-border-subtle shadow-2xl animate-fade-in-scale" onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-5 pb-3">
          <h3 className="text-base font-black">{title}</h3>
          {subtitle && <p className="text-[11px] text-text-dim mt-0.5 truncate">{subtitle}</p>}
        </div>

        {/* Type tabs - segmented control style */}
        <div className="px-5 pb-4">
          <div className="flex rounded-xl bg-main p-0.5">
            {types.map(tp => (
              <button key={tp.key} onClick={() => setScheduleType(tp.key)}
                className={`flex-1 py-1.5 rounded-lg text-[11px] font-bold transition-colors duration-200 ${
                  scheduleType === tp.key
                    ? "bg-surface text-brand shadow-sm"
                    : "text-text-dim/60 hover:text-text-dim"
                }`}>
                {tp.label}
              </button>
            ))}
          </div>
        </div>

        {/* Config area - fixed height */}
        <div className="px-5 h-[88px] flex items-center">
          {scheduleType === "interval_min" && (
            <div className="flex items-center gap-2 text-sm">
              <span className="text-text-dim">{t("scheduler.every")}</span>
              <input type="number" min={1} max={59} value={intervalMin}
                onChange={e => setIntervalMin(Math.max(1, Math.min(59, +e.target.value)))} className={num} />
              <span className="text-text-dim">{t("scheduler.minutes")}</span>
            </div>
          )}
          {scheduleType === "interval_hour" && (
            <div className="flex items-center gap-2 text-sm">
              <span className="text-text-dim">{t("scheduler.every")}</span>
              <input type="number" min={1} max={23} value={intervalHour}
                onChange={e => setIntervalHour(Math.max(1, Math.min(23, +e.target.value)))} className={num} />
              <span className="text-text-dim">{t("scheduler.hours")}</span>
            </div>
          )}
          {(scheduleType === "daily" || scheduleType === "weekday") && (
            <div className="flex items-center gap-2 text-sm">
              <span className="text-text-dim">{scheduleType === "weekday" ? t("scheduler.weekdays_at") : t("scheduler.daily_at")}</span>
              {timeSelect}
            </div>
          )}
          {scheduleType === "weekly" && (
            <div className="flex flex-col gap-3 w-full">
              <div className="flex justify-between">
                {wdShort.map((d, i) => (
                  <button key={i} onClick={() => setWeekday(i + 1)}
                    className={`w-8 h-8 rounded-full text-[11px] font-bold transition-colors ${
                      weekday === i + 1 ? "bg-brand text-white" : "text-text-dim hover:bg-main"
                    }`}>{d}</button>
                ))}
              </div>
              <div className="flex items-center gap-2 text-sm">
                <span className="text-text-dim">{t("scheduler.at")}</span>
                {timeSelect}
              </div>
            </div>
          )}
          {scheduleType === "monthly" && (
            <div className="flex items-center gap-2 text-sm">
              <span className="text-text-dim">{t("scheduler.every_month_on")}</span>
              <input type="number" min={1} max={28} value={monthDay}
                onChange={e => setMonthDay(Math.max(1, Math.min(28, +e.target.value)))} className={num} />
              <span className="text-text-dim">{t("scheduler.day_suffix")}</span>
              {timeSelect}
            </div>
          )}
          {scheduleType === "custom" && (() => {
            const fields = customCron.split(/\s+/);
            while (fields.length < 5) fields.push("*");
            const updateField = (idx: number, v: string) => {
              const f = [...fields]; f[idx] = v;
              setCustomCron(f.slice(0, 5).join(" "));
            };
            const opts: string[][] = [
              ["*", "*/5", "*/10", "*/15", "*/30", ...Array.from({ length: 60 }, (_, i) => String(i))],
              ["*", "*/2", "*/4", "*/6", "*/12", ...Array.from({ length: 24 }, (_, i) => String(i))],
              ["*", ...Array.from({ length: 31 }, (_, i) => String(i + 1))],
              ["*", ...Array.from({ length: 12 }, (_, i) => String(i + 1))],
              ["*", "0", "1", "2", "3", "4", "5", "6", "1-5"],
            ];
            return (
              <div className="grid grid-cols-5 gap-2 w-full">
                {cronFieldHeaders.map((h, i) => (
                  <div key={i}>
                    <p className="text-[9px] font-bold text-text-dim/50 text-center mb-1">{h}</p>
                    <select value={fields[i] || "*"} onChange={e => updateField(i, e.target.value)}
                      className="w-full h-9 rounded-lg border border-border-subtle bg-main text-xs font-mono text-center outline-none focus:border-brand transition-colors">
                      {opts[i].map(v => <option key={v} value={v}>{v}</option>)}
                    </select>
                  </div>
                ))}
              </div>
            );
          })()}
        </div>

        {/* Timezone picker */}
        <div className="mx-5 mt-1 mb-2 flex items-center gap-2">
          <label className="text-[10px] font-bold text-text-dim/50 uppercase shrink-0">{t("scheduler.timezone", { defaultValue: "Timezone" })}</label>
          <select value={timezone} onChange={e => setTimezone(e.target.value)}
            className="flex-1 h-8 rounded-lg border border-border-subtle bg-main px-2 text-xs outline-none focus:border-brand transition-colors">
            {timezoneOptions.map(tz => <option key={tz} value={tz}>{tz.replace(/_/g, " ")}</option>)}
          </select>
        </div>

        {/* Result bar */}
        <div className="mx-5 mt-1 mb-4 flex items-center justify-between rounded-xl bg-main px-4 py-2.5">
          <span className={`text-xs font-medium ${cronValid ? "text-text-dim" : "text-error"}`}>
            {describeCron(previewCron)}{timezone !== "UTC" ? ` (${timezone.split("/").pop()?.replace(/_/g, " ")})` : " (UTC)"}
          </span>
          <code className={`text-[11px] font-mono font-bold px-2 py-0.5 rounded-md ${
            cronValid ? "bg-brand/10 text-brand" : "bg-error/10 text-error"
          }`}>{previewCron}</code>
        </div>

        {/* Actions */}
        <div className="flex gap-2 px-5 pb-5">
          <Button variant="primary" className="flex-1" onClick={() => onSave(previewCron, timezone)} disabled={!cronValid}>{t("common.save")}</Button>
          <Button variant="secondary" className="flex-1" onClick={onClose}>{t("common.cancel")}</Button>
        </div>
      </div>
    </div>
  );
}
