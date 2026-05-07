import React, { useRef, useState, useEffect } from "react";
import { useTranslation } from "react-i18next";

interface NodeEditorProps {
  node: { id: string; type: string; data?: { label?: string } } | null;
  onUpdate: (id: string, data: { label: string }) => void;
}

export const NodeEditor = React.memo(function NodeEditor({ node, onUpdate }: NodeEditorProps) {
  const { t } = useTranslation();
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingRef = useRef<{ id: string; label: string } | null>(null);
  const [localLabel, setLocalLabel] = useState(node?.data?.label ?? "");
  const prevNodeId = useRef<string | null>(node?.id ?? null);

  useEffect(() => {
    if (node) {
      if (prevNodeId.current && prevNodeId.current !== node.id) {
        const pending = pendingRef.current;
        if (pending && pending.id === prevNodeId.current) {
          onUpdate(pending.id, { label: pending.label });
        }
        pendingRef.current = null;
      }
      if (timerRef.current) clearTimeout(timerRef.current);
      prevNodeId.current = node.id;
      setLocalLabel(node.data?.label ?? "");
    }
  }, [node?.id, onUpdate]);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
      const pending = pendingRef.current;
      if (pending) onUpdate(pending.id, { label: pending.label });
    };
  }, [onUpdate]);

  if (!node) return (
    <div className="h-full flex items-center justify-center text-text-dim/40 font-bold uppercase tracking-widest text-[10px]">
      {t("common.no_data")}
    </div>
  );

  return (
    <div className="p-6">
      <h3 className="text-[10px] font-black uppercase tracking-widest text-text-dim mb-6">{t("common.properties")}</h3>
      <div className="space-y-4">
        <div>
          <label htmlFor="node-label" className="text-[10px] font-black uppercase text-brand mb-1.5 block">{t("common.label")}</label>
          <input
            id="node-label"
            value={localLabel}
            onChange={(e) => {
              const value = e.target.value;
              setLocalLabel(value);
              if (timerRef.current) clearTimeout(timerRef.current);
              pendingRef.current = { id: node.id, label: value };
              timerRef.current = setTimeout(() => {
                onUpdate(node.id, { label: value });
                pendingRef.current = null;
              }, 300);
            }}
            className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-colors"
          />
        </div>
        <div>
          <span className="text-[10px] font-black uppercase text-brand mb-1.5 block">{t("common.type")}</span>
          <div className="w-full rounded-xl border border-border-subtle bg-main/50 px-4 py-2 text-sm text-text-dim">
            {node.type}
          </div>
        </div>
      </div>
    </div>
  );
});
