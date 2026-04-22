import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type MediaProvider,
  type MediaImageResult,
  type SpeechResult,
  type MediaMusicResult,
  type MediaVideoStatus,
} from "../lib/http/client";
import { useMediaProviders, useVideoTask } from "../lib/queries/media";
import {
  useGenerateImage,
  useSynthesizeSpeech,
  useSubmitVideo,
  useGenerateMusic,
} from "../lib/mutations/media";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { useUIStore } from "../lib/store";
import {
  Image as ImageIcon,
  Mic,
  Film,
  Music,
  Sparkles,
  AlertCircle,
  CheckCircle,
  XCircle,
  Loader2,
} from "lucide-react";

type MediaTab = "image" | "speech" | "video" | "music";
type OnToast = (msg: string, kind?: "success" | "error") => void;

const CAPABILITY_TAB: Record<string, MediaTab> = {
  image_generation: "image",
  text_to_speech: "speech",
  video_generation: "video",
  music_generation: "music",
};

const TAB_ICONS: Record<MediaTab, React.ReactNode> = {
  image: <ImageIcon className="w-3.5 h-3.5" />,
  speech: <Mic className="w-3.5 h-3.5" />,
  video: <Film className="w-3.5 h-3.5" />,
  music: <Music className="w-3.5 h-3.5" />,
};

const inputClass =
  "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand";
const textareaClass =
  "w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand resize-y min-h-[88px]";

export function MediaPage() {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [activeTab, setActiveTab] = useState<MediaTab>("image");

  const providersQuery = useMediaProviders();

  const providers = providersQuery.data ?? [];
  const configuredProviders = useMemo(() => providers.filter((p) => p.configured), [providers]);

  const imageProviders = useMemo(
    () => configuredProviders.filter((p) => p.capabilities.includes("image_generation")),
    [configuredProviders],
  );
  const speechProviders = useMemo(
    () => configuredProviders.filter((p) => p.capabilities.includes("text_to_speech")),
    [configuredProviders],
  );
  const videoProviders = useMemo(
    () => configuredProviders.filter((p) => p.capabilities.includes("video_generation")),
    [configuredProviders],
  );
  const musicProviders = useMemo(
    () => configuredProviders.filter((p) => p.capabilities.includes("music_generation")),
    [configuredProviders],
  );

  return (
    <div className="flex flex-col gap-6 transition-colors duration-300">
      <PageHeader
        badge={t("media.section")}
        title={t("media.title")}
        subtitle={t("media.subtitle")}
        icon={<Sparkles className="h-4 w-4" />}
        isFetching={providersQuery.isFetching}
        onRefresh={() => providersQuery.refetch()}
        helpText={t("media.help")}
      />

      {providersQuery.isError && (
        <div className="flex items-center gap-3 p-4 rounded-2xl bg-error/5 border border-error/20 text-error">
          <AlertCircle className="w-5 h-5 shrink-0" />
          <p className="text-sm">{t("media.load_error")}</p>
        </div>
      )}

      {/* Provider status grid */}
      <ProviderStatusGrid providers={providers} />

      {/* Tab bar */}
      <div className="flex gap-1 rounded-xl border border-border-subtle bg-surface p-1 flex-wrap">
        <TabButton tab="image" active={activeTab} onClick={setActiveTab} icon={TAB_ICONS.image}>
          {t("media.tab_image")}
        </TabButton>
        <TabButton tab="speech" active={activeTab} onClick={setActiveTab} icon={TAB_ICONS.speech}>
          {t("media.tab_speech")}
        </TabButton>
        <TabButton tab="video" active={activeTab} onClick={setActiveTab} icon={TAB_ICONS.video}>
          {t("media.tab_video")}
        </TabButton>
        <TabButton tab="music" active={activeTab} onClick={setActiveTab} icon={TAB_ICONS.music}>
          {t("media.tab_music")}
        </TabButton>
      </div>

      {/* Active panel */}
      <div className="rounded-2xl border border-border-subtle bg-surface p-5">
        {activeTab === "image" && (
          <ImagePanel providers={imageProviders} onToast={addToast} />
        )}
        {activeTab === "speech" && (
          <SpeechPanel providers={speechProviders} onToast={addToast} />
        )}
        {activeTab === "video" && (
          <VideoPanel providers={videoProviders} onToast={addToast} />
        )}
        {activeTab === "music" && (
          <MusicPanel providers={musicProviders} onToast={addToast} />
        )}
      </div>
    </div>
  );
}

// ─── Provider status grid ─────────────────────────────────────────────

function ProviderStatusGrid({ providers }: { providers: MediaProvider[] }) {
  const { t } = useTranslation();
  if (providers.length === 0) {
    return (
      <div className="rounded-2xl border border-border-subtle bg-surface p-5 text-sm text-text-dim">
        {t("media.no_providers")}
      </div>
    );
  }
  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
      {providers.map((p) => (
        <div
          key={p.name}
          className={`rounded-2xl border p-4 transition-colors ${
            p.configured
              ? "border-success/30 bg-success/5"
              : "border-border-subtle bg-surface opacity-70"
          }`}
        >
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              {p.configured ? (
                <CheckCircle className="w-4 h-4 text-success" />
              ) : (
                <XCircle className="w-4 h-4 text-text-dim" />
              )}
              <span className="text-sm font-bold">{p.name}</span>
            </div>
            <Badge variant={p.configured ? "success" : "default"}>
              {p.configured ? t("media.configured") : t("media.not_configured")}
            </Badge>
          </div>
          <div className="flex flex-wrap gap-1">
            {p.capabilities.length === 0 ? (
              <span className="text-[11px] text-text-dim">{t("media.no_capabilities")}</span>
            ) : (
              p.capabilities.map((cap) => (
                <span
                  key={cap}
                  className="px-1.5 py-0.5 rounded-md bg-main text-[10px] font-medium text-text-dim"
                >
                  {t(`media.cap_${CAPABILITY_TAB[cap] ?? cap}`, { defaultValue: cap })}
                </span>
              ))
            )}
          </div>
        </div>
      ))}
    </div>
  );
}

// ─── Tab button ───────────────────────────────────────────────────────

function TabButton({
  tab,
  active,
  onClick,
  icon,
  children,
}: {
  tab: MediaTab;
  active: MediaTab;
  onClick: (t: MediaTab) => void;
  icon: React.ReactNode;
  children: React.ReactNode;
}) {
  const isActive = tab === active;
  return (
    <button
      onClick={() => onClick(tab)}
      className={`flex items-center gap-1.5 px-3 py-2 rounded-lg text-xs font-bold transition-colors ${
        isActive
          ? "bg-brand text-white shadow-sm"
          : "text-text-dim hover:text-text hover:bg-main"
      }`}
    >
      {icon}
      {children}
    </button>
  );
}

// ─── Provider selector ────────────────────────────────────────────────

function ProviderSelect({
  value,
  onChange,
  providers,
  placeholder,
}: {
  value: string;
  onChange: (v: string) => void;
  providers: MediaProvider[];
  placeholder: string;
}) {
  return (
    <select value={value} onChange={(e) => onChange(e.target.value)} className={inputClass}>
      <option value="">{placeholder}</option>
      {providers.map((p) => (
        <option key={p.name} value={p.name}>
          {p.name}
        </option>
      ))}
    </select>
  );
}

// ─── Image panel ──────────────────────────────────────────────────────

function ImagePanel({
  providers,
  onToast,
}: {
  providers: MediaProvider[];
  onToast: OnToast;
}) {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState("");
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [count, setCount] = useState(1);
  const [aspect, setAspect] = useState("");
  const [result, setResult] = useState<MediaImageResult | null>(null);

  const mut = useGenerateImage();

  return (
    <form
      onSubmit={(e: FormEvent) => {
        e.preventDefault();
        if (!prompt.trim()) return;
        mut.mutate(
          {
            prompt,
            provider: provider || undefined,
            model: model || undefined,
            count: count || undefined,
            aspect_ratio: aspect || undefined,
          },
          {
            onSuccess: (data) => {
              setResult(data);
              onToast(t("media.image_done"), "success");
            },
            onError: (err: Error) => onToast(err.message || t("common.error"), "error"),
          },
        );
      }}
      className="flex flex-col gap-4"
    >
      <PanelHeader icon={<ImageIcon className="w-4 h-4" />} title={t("media.image_title")} desc={t("media.image_desc")} />
      <FormField label={t("media.prompt")}>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={t("media.image_prompt_placeholder")}
          className={textareaClass}
          required
        />
      </FormField>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
        <FormField label={t("media.provider")}>
          <ProviderSelect value={provider} onChange={setProvider} providers={providers} placeholder={t("media.auto_detect")} />
        </FormField>
        <FormField label={t("media.model")}>
          <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder={t("media.model_placeholder")} />
        </FormField>
        <FormField label={t("media.count")}>
          <input
            type="number"
            min={1}
            max={4}
            value={count}
            onChange={(e) => setCount(Number(e.target.value))}
            className={inputClass}
          />
        </FormField>
        <FormField label={t("media.aspect_ratio")}>
          <Input value={aspect} onChange={(e) => setAspect(e.target.value)} placeholder={t("media.aspect_ratio_placeholder", { defaultValue: "1:1" })} />
        </FormField>
      </div>
      <div className="flex items-center gap-3">
        <Button type="submit" variant="primary" isLoading={mut.isPending} disabled={!prompt.trim() || mut.isPending}>
          {t("media.generate")}
        </Button>
        {providers.length === 0 && <NoProviderHint tab="image" />}
      </div>

      {result && (
        <ResultBlock provider={result.provider} model={result.model}>
          {result.revised_prompt && (
            <p className="text-xs text-text-dim mb-3 italic">{t("media.revised_prompt")}: {result.revised_prompt}</p>
          )}
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {result.images.map((img, i) => (
              <a
                key={i}
                href={img.url}
                target="_blank"
                rel="noreferrer"
                className="block rounded-xl overflow-hidden border border-border-subtle hover:border-brand/40 transition-colors"
              >
                {img.url ? (
                  <img src={img.url} alt={t("media.generated_alt", { index: i + 1, defaultValue: "generated {{index}}" })} className="w-full h-auto" />
                ) : (
                  <img src={`data:image/png;base64,${img.data_base64}`} alt={t("media.generated_alt", { index: i + 1, defaultValue: "generated {{index}}" })} className="w-full h-auto" />
                )}
              </a>
            ))}
          </div>
        </ResultBlock>
      )}
    </form>
  );
}

// ─── Speech panel ─────────────────────────────────────────────────────

function SpeechPanel({
  providers,
  onToast,
}: {
  providers: MediaProvider[];
  onToast: OnToast;
}) {
  const { t } = useTranslation();
  const [text, setText] = useState("");
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [voice, setVoice] = useState("");
  const [format, setFormat] = useState("mp3");
  const [speed, setSpeed] = useState(1);
  const [result, setResult] = useState<SpeechResult | null>(null);

  const mut = useSynthesizeSpeech();

  return (
    <form
      onSubmit={(e: FormEvent) => {
        e.preventDefault();
        if (!text.trim()) return;
        mut.mutate(
          {
            text,
            provider: provider || undefined,
            model: model || undefined,
            voice: voice || undefined,
            format: format || undefined,
            speed: speed || undefined,
          },
          {
            onSuccess: (data) => {
              setResult(data);
              onToast(t("media.speech_done"), "success");
            },
            onError: (err: Error) => onToast(err.message || t("common.error"), "error"),
          },
        );
      }}
      className="flex flex-col gap-4"
    >
      <PanelHeader icon={<Mic className="w-4 h-4" />} title={t("media.speech_title")} desc={t("media.speech_desc")} />
      <FormField label={t("media.text")}>
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={t("media.speech_text_placeholder")}
          className={textareaClass}
          required
        />
      </FormField>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-3">
        <FormField label={t("media.provider")}>
          <ProviderSelect value={provider} onChange={setProvider} providers={providers} placeholder={t("media.auto_detect")} />
        </FormField>
        <FormField label={t("media.model")}>
          <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder={t("media.model_placeholder")} />
        </FormField>
        <FormField label={t("media.voice")}>
          <Input value={voice} onChange={(e) => setVoice(e.target.value)} placeholder={t("media.voice_placeholder")} />
        </FormField>
        <FormField label={t("media.format")}>
          <select value={format} onChange={(e) => setFormat(e.target.value)} className={inputClass}>
            <option value="mp3">{t("media.format_mp3", { defaultValue: "mp3" })}</option>
            <option value="wav">{t("media.format_wav", { defaultValue: "wav" })}</option>
            <option value="flac">{t("media.format_flac", { defaultValue: "flac" })}</option>
            <option value="ogg">{t("media.format_ogg", { defaultValue: "ogg" })}</option>
            <option value="opus">{t("media.format_opus", { defaultValue: "opus" })}</option>
            <option value="aac">{t("media.format_aac", { defaultValue: "aac" })}</option>
          </select>
        </FormField>
        <FormField label={t("media.speed")}>
          <input
            type="number"
            min={0.25}
            max={4}
            step={0.05}
            value={speed}
            onChange={(e) => setSpeed(Number(e.target.value))}
            className={inputClass}
          />
        </FormField>
      </div>
      <div className="flex items-center gap-3">
        <Button type="submit" variant="primary" isLoading={mut.isPending} disabled={!text.trim() || mut.isPending}>
          {t("media.synthesize")}
        </Button>
        {providers.length === 0 && <NoProviderHint tab="speech" />}
      </div>

      {result && (
        <ResultBlock provider={result.provider} model={result.model} duration={result.duration_ms}>
          <audio controls src={result.url} className="w-full" />
          <a href={result.url} target="_blank" rel="noreferrer" className="text-xs text-brand hover:underline mt-2 inline-block">
            {t("media.download")} ({result.format})
          </a>
        </ResultBlock>
      )}
    </form>
  );
}

// ─── Video panel ──────────────────────────────────────────────────────

function VideoPanel({
  providers,
  onToast,
}: {
  providers: MediaProvider[];
  onToast: OnToast;
}) {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState("");
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  // Local draft shown immediately after submission, before the first poll
  // returns. Once the query has data we derive status from the query instead —
  // keeping a mirrored copy of query data in state is a React anti-pattern
  // and can race the first fetch for a new taskId.
  const [submittedDraft, setSubmittedDraft] = useState<MediaVideoStatus | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [taskProvider, setTaskProvider] = useState<string | null>(null);
  const completionToastShown = useRef<string | null>(null);
  const errorToastShown = useRef<string | null>(null);

  const submit = useSubmitVideo();
  const videoTaskQuery = useVideoTask(
    taskId && taskProvider ? { taskId, provider: taskProvider } : null,
    {
      enabled: Boolean(taskId && taskProvider), // Only poll after a submission creates a task.
      refetchInterval: 5_000,
    },
  );

  const status: MediaVideoStatus | null = videoTaskQuery.data ?? submittedDraft;
  const statusState = status?.status;
  const statusError = status?.error;
  const statusResult = status?.result;

  useEffect(() => {
    if (!videoTaskQuery.isError) return;
    const message = videoTaskQuery.error instanceof Error ? videoTaskQuery.error.message : t("common.error");
    if (errorToastShown.current === message) return;
    errorToastShown.current = message;
    onToast(message, "error");
  }, [videoTaskQuery.error, videoTaskQuery.isError, onToast, t]);

  useEffect(() => {
    if (!statusState) return;
    if (statusState === "completed") {
      if (completionToastShown.current === taskId) return;
      completionToastShown.current = taskId;
      onToast(t("media.video_done"), "success");
      return;
    }
    if (statusError) {
      if (errorToastShown.current === statusError) return;
      errorToastShown.current = statusError;
      onToast(statusError, "error");
    }
  }, [onToast, statusError, statusState, t, taskId]);

  const isPolling = !!(taskId && taskProvider)
    && !!statusState
    && statusState !== "completed"
    && statusState !== "failed"
    && !statusError;

  return (
    <form
      onSubmit={(e: FormEvent) => {
        e.preventDefault();
        if (!prompt.trim()) return;
        submit.mutate(
          {
            prompt,
            provider: provider || undefined,
            model: model || undefined,
          },
          {
            onSuccess: (data) => {
              setSubmittedDraft({ status: "submitted", task_id: data.task_id });
              setTaskId(data.task_id);
              setTaskProvider(data.provider);
              completionToastShown.current = null;
              errorToastShown.current = null;
              onToast(t("media.video_submitted"), "success");
            },
            onError: (err: Error) => onToast(err.message || t("common.error"), "error"),
          },
        );
      }}
      className="flex flex-col gap-4"
    >
      <PanelHeader icon={<Film className="w-4 h-4" />} title={t("media.video_title")} desc={t("media.video_desc")} />
      <FormField label={t("media.prompt")}>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={t("media.video_prompt_placeholder")}
          className={textareaClass}
          required
        />
      </FormField>
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-2 gap-3">
        <FormField label={t("media.provider")}>
          <ProviderSelect value={provider} onChange={setProvider} providers={providers} placeholder={t("media.auto_detect")} />
        </FormField>
        <FormField label={t("media.model")}>
          <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder={t("media.model_placeholder")} />
        </FormField>
      </div>
      <div className="flex items-center gap-3">
        <Button type="submit" variant="primary" isLoading={submit.isPending || isPolling} disabled={!prompt.trim() || submit.isPending || isPolling}>
          {isPolling ? t("media.polling") : t("media.generate_video")}
        </Button>
        {providers.length === 0 && <NoProviderHint tab="video" />}
        {taskId && (
          <span className="text-xs text-text-dim">
            {t("media.task_id")}: <code className="px-1.5 py-0.5 rounded bg-main">{taskId}</code>
            {taskProvider && <> · <span>{taskProvider}</span></>}
          </span>
        )}
      </div>

      {status && (
        <ResultBlock
          provider={statusResult?.provider ?? taskProvider ?? ""}
          model={statusResult?.model ?? ""}
        >
          <div className="flex items-center gap-2 mb-3">
            <span className="text-xs font-bold text-text-dim">{t("media.status")}:</span>
            <StatusBadge status={statusState ?? "submitted"} />
          </div>
          {statusState === "completed" && statusResult && (
            <div className="flex flex-col gap-2">
              <video controls src={statusResult.file_url} className="w-full rounded-xl border border-border-subtle" />
              <div className="text-xs text-text-dim flex flex-wrap gap-3">
                {statusResult.width && statusResult.height && (
                  <span>{statusResult.width}×{statusResult.height}</span>
                )}
                {statusResult.duration_secs != null && <span>{statusResult.duration_secs.toFixed(1)}s</span>}
                <a href={statusResult.file_url} target="_blank" rel="noreferrer" className="text-brand hover:underline">
                  {t("media.download")}
                </a>
              </div>
            </div>
          )}
          {statusState !== "completed" && statusState !== "failed" && !statusError && (
            <div className="flex items-center gap-2 text-xs text-text-dim">
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
              <span>{t("media.video_polling")}</span>
            </div>
          )}
          {statusError && <p className="text-xs text-error">{statusError}</p>}
        </ResultBlock>
      )}
    </form>
  );
}

// ─── Music panel ──────────────────────────────────────────────────────

function MusicPanel({
  providers,
  onToast,
}: {
  providers: MediaProvider[];
  onToast: OnToast;
}) {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState("");
  const [lyrics, setLyrics] = useState("");
  const [provider, setProvider] = useState("");
  const [model, setModel] = useState("");
  const [instrumental, setInstrumental] = useState(false);
  const [result, setResult] = useState<MediaMusicResult | null>(null);

  const mut = useGenerateMusic();

  const canSubmit = !!prompt.trim() || !!lyrics.trim();

  return (
    <form
      onSubmit={(e: FormEvent) => {
        e.preventDefault();
        if (!canSubmit) return;
        mut.mutate(
          {
            prompt: prompt || undefined,
            lyrics: lyrics || undefined,
            provider: provider || undefined,
            model: model || undefined,
            instrumental,
          },
          {
            onSuccess: (data) => {
              setResult(data);
              onToast(t("media.music_done"), "success");
            },
            onError: (err: Error) => onToast(err.message || t("common.error"), "error"),
          },
        );
      }}
      className="flex flex-col gap-4"
    >
      <PanelHeader icon={<Music className="w-4 h-4" />} title={t("media.music_title")} desc={t("media.music_desc")} />
      <FormField label={t("media.music_prompt")}>
        <textarea
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder={t("media.music_prompt_placeholder")}
          className={textareaClass}
        />
      </FormField>
      <FormField label={t("media.lyrics")}>
        <textarea
          value={lyrics}
          onChange={(e) => setLyrics(e.target.value)}
          placeholder={t("media.lyrics_placeholder")}
          className={textareaClass}
        />
      </FormField>
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <FormField label={t("media.provider")}>
          <ProviderSelect value={provider} onChange={setProvider} providers={providers} placeholder={t("media.auto_detect")} />
        </FormField>
        <FormField label={t("media.model")}>
          <Input value={model} onChange={(e) => setModel(e.target.value)} placeholder={t("media.model_placeholder")} />
        </FormField>
        <FormField label={t("media.instrumental")}>
          <label className="flex items-center gap-2 mt-2">
            <input type="checkbox" checked={instrumental} onChange={(e) => setInstrumental(e.target.checked)} />
            <span className="text-sm text-text-dim">{t("media.instrumental_desc")}</span>
          </label>
        </FormField>
      </div>
      <div className="flex items-center gap-3">
        <Button type="submit" variant="primary" isLoading={mut.isPending} disabled={!canSubmit || mut.isPending}>
          {t("media.compose")}
        </Button>
        {providers.length === 0 && <NoProviderHint tab="music" />}
      </div>

      {result && (
        <ResultBlock provider={result.provider} model={result.model} duration={result.duration_ms}>
          <audio controls src={result.url} className="w-full" />
          <a href={result.url} target="_blank" rel="noreferrer" className="text-xs text-brand hover:underline mt-2 inline-block">
            {t("media.download")} ({result.format})
          </a>
        </ResultBlock>
      )}
    </form>
  );
}

// ─── Shared subcomponents ─────────────────────────────────────────────

function PanelHeader({ icon, title, desc }: { icon: React.ReactNode; title: string; desc: string }) {
  return (
    <div className="flex items-start gap-3 pb-3 border-b border-border-subtle">
      <div className="p-2 rounded-lg bg-brand/10 text-brand shrink-0">{icon}</div>
      <div>
        <h3 className="text-sm font-extrabold">{title}</h3>
        <p className="text-xs text-text-dim mt-0.5">{desc}</p>
      </div>
    </div>
  );
}

function FormField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-[11px] font-bold uppercase tracking-wider text-text-dim">{label}</label>
      {children}
    </div>
  );
}

function ResultBlock({
  children,
  provider,
  model,
  duration,
}: {
  children: React.ReactNode;
  provider: string;
  model: string;
  duration?: number;
}) {
  const { t } = useTranslation();
  return (
    <div className="mt-2 rounded-xl border border-border-subtle bg-main/50 p-4">
      <div className="flex items-center gap-3 mb-3 flex-wrap">
        <Badge variant="success">{t("media.result")}</Badge>
        {provider && <span className="text-xs text-text-dim">{provider}</span>}
        {model && <span className="text-xs text-text-dim">· {model}</span>}
        {duration != null && <span className="text-xs text-text-dim">· {(duration / 1000).toFixed(1)}s</span>}
      </div>
      {children}
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const { t } = useTranslation();
  const variant: "success" | "error" | "default" =
    status === "completed"
      ? "success"
      : status === "failed"
        ? "error"
        : "default";
  return <Badge variant={variant}>{t(`media.status_${status}`, { defaultValue: status })}</Badge>;
}

function NoProviderHint({ tab }: { tab: MediaTab }) {
  const { t } = useTranslation();
  return (
    <span className="text-xs text-warning flex items-center gap-1.5">
      <AlertCircle className="w-3.5 h-3.5" />
      {t(`media.no_provider_${tab}`)}
    </span>
  );
}
