import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Download, Loader2 } from "lucide-react";
import { Modal } from "./ui/Modal";
import { useUIStore } from "../lib/store";

interface TomlViewerProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  /** Resolved TOML text. Pass undefined while still loading. */
  toml?: string;
  /** Optional second tab (e.g. Markdown). Hidden when undefined. */
  markdown?: string;
  /** Filename suggested by the Download button. Defaults to "manifest.toml". */
  downloadName?: string;
  /** Surfaced near the buttons when the fetch errored. */
  error?: string | null;
}

// Reusable read-only viewer for TOML/Markdown bodies. Used by HandsPage
// and ConfigPage to show the canonical on-disk representation of a hand
// or the kernel config without giving up the structured editor surface.
export function TomlViewer({
  isOpen,
  onClose,
  title,
  toml,
  markdown,
  downloadName = "manifest.toml",
  error,
}: TomlViewerProps) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const [tab, setTab] = useState<"toml" | "markdown">("toml");

  const body = tab === "toml" ? toml : markdown;
  const loading = body === undefined && !error;

  const onCopy = async () => {
    if (!body) return;
    try {
      await navigator.clipboard.writeText(body);
      addToast(t("toml_viewer.copied"), "success");
    } catch {
      addToast(t("toml_viewer.copy_failed"), "error");
    }
  };

  const onDownload = () => {
    if (!body) return;
    const filename =
      tab === "markdown" ? downloadName.replace(/\.toml$/i, ".md") : downloadName;
    const mime = tab === "markdown" ? "text/markdown" : "application/toml";
    const blob = new Blob([body], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={title} size="3xl">
      <div className="p-5 space-y-3">
        <div className="flex items-center justify-between gap-2">
          {markdown !== undefined ? (
            <div className="flex gap-1">
              <button
                type="button"
                onClick={() => setTab("toml")}
                className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                  tab === "toml" ? "bg-brand text-white" : "text-text-dim hover:text-text"
                }`}
              >
                {t("toml_viewer.tab_toml")}
              </button>
              <button
                type="button"
                onClick={() => setTab("markdown")}
                className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                  tab === "markdown" ? "bg-brand text-white" : "text-text-dim hover:text-text"
                }`}
              >
                {t("toml_viewer.tab_markdown")}
              </button>
            </div>
          ) : (
            <span className="text-[10px] font-bold uppercase text-text-dim">
              {t("toml_viewer.tab_toml")}
            </span>
          )}
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onCopy}
              disabled={!body}
              className="text-[10px] font-bold text-text-dim hover:text-brand disabled:opacity-40"
              title={t("toml_viewer.copy")}
              aria-label={t("toml_viewer.copy")}
            >
              <Copy className="w-3.5 h-3.5" />
            </button>
            <button
              type="button"
              onClick={onDownload}
              disabled={!body}
              className="text-[10px] font-bold text-text-dim hover:text-brand disabled:opacity-40"
              title={t("toml_viewer.download")}
              aria-label={t("toml_viewer.download")}
            >
              <Download className="w-3.5 h-3.5" />
            </button>
          </div>
        </div>
        {error ? (
          <p className="text-xs text-error rounded-lg border border-error/30 bg-error/5 px-3 py-2">
            {error}
          </p>
        ) : loading ? (
          <div className="flex items-center gap-2 text-xs text-text-dim">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            {t("toml_viewer.loading")}
          </div>
        ) : (
          <div className="max-h-[65vh] overflow-x-auto overflow-y-auto rounded-xl border border-border-subtle bg-main">
            <pre className="min-w-full px-3 py-2 text-[11px] font-mono text-text whitespace-pre-wrap">
              {body}
            </pre>
          </div>
        )}
      </div>
    </Modal>
  );
}
