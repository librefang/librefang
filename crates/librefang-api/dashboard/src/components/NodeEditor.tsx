import { useTranslation } from "react-i18next";

export function NodeEditor({ node, onUpdate }: any) {
  const { t } = useTranslation();

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
          <label className="text-[10px] font-black uppercase text-brand mb-1.5 block">{t("common.label")}</label>
          <input 
            value={node.data.label} 
            onChange={(e) => onUpdate(node.id, { label: e.target.value })}
            className="w-full rounded-xl border border-border-subtle bg-main px-4 py-2 text-sm focus:border-brand outline-none transition-all"
          />
        </div>
        <div>
          <label className="text-[10px] font-black uppercase text-brand mb-1.5 block">{t("common.type")}</label>
          <input 
            value={node.type} 
            readOnly
            className="w-full rounded-xl border border-border-subtle bg-main/50 px-4 py-2 text-sm text-text-dim cursor-not-allowed"
          />
        </div>
      </div>
    </div>
  );
}
