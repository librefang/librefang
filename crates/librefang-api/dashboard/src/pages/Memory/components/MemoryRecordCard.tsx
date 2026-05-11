import { useTranslation } from "react-i18next";
import { Edit2, Trash2 } from "lucide-react";
import { Card } from "../../../components/ui/Card";
import { Badge } from "../../../components/ui/Badge";
import { Button } from "../../../components/ui/Button";
import { MarkdownContent } from "../../../components/ui/MarkdownContent";
import { formatDateTime } from "../../../lib/datetime";
import type { MemoryItem } from "../../../api";

interface Props {
  memory: MemoryItem;
  onEdit: (memory: MemoryItem) => void;
  onDelete: (id: string) => void;
}

export function MemoryRecordCard({ memory, onEdit, onDelete }: Props) {
  const { t } = useTranslation();
  return (
    <Card hover padding="md">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-1 sm:gap-2 mb-2">
        <div className="flex items-center gap-2 min-w-0 flex-wrap">
          <h2 className="text-xs sm:text-sm font-black truncate font-mono max-w-45 sm:max-w-none">
            {memory.id}
          </h2>
          <Badge
            variant={
              memory.level === "user"
                ? "info"
                : memory.level === "session"
                ? "warning"
                : memory.level === "agent"
                ? "brand"
                : "default"
            }
          >
            {memory.level || t("memory.session", { defaultValue: "session" })}
          </Badge>
          {memory.source && <Badge variant="default">{memory.source}</Badge>}
          {memory.confidence != null && (
            <Badge
              variant={
                memory.confidence > 0.7
                  ? "success"
                  : memory.confidence > 0.3
                  ? "warning"
                  : "default"
              }
            >
              {Math.round(memory.confidence * 100)}%
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1 shrink-0 self-end sm:self-auto">
          <Button variant="ghost" size="sm" onClick={() => onEdit(memory)}>
            <Edit2 className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            className="text-error! hover:bg-error/10!"
            onClick={() => onDelete(memory.id)}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
      <MarkdownContent className="text-xs text-text-dim leading-relaxed h-16 overflow-y-auto">
        {memory.content || t("common.no_data")}
      </MarkdownContent>
      <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[10px] text-text-dim/50">
        {memory.created_at && (
          <span>
            {t("memory.created")}: {formatDateTime(memory.created_at)}
          </span>
        )}
        {memory.accessed_at && (
          <span>
            {t("memory.last_access", { defaultValue: "Last access" })}:{" "}
            {formatDateTime(memory.accessed_at)}
          </span>
        )}
        {memory.access_count != null && memory.access_count > 0 && (
          <span>
            {t("memory.access_count", { defaultValue: "Accessed" })}: {memory.access_count}x
          </span>
        )}
        {memory.agent_id && (
          <span>
            {t("memory.agent_label", { defaultValue: "Agent:" })}{" "}
            <span className="font-mono">{memory.agent_id.slice(0, 8)}</span>
          </span>
        )}
        {memory.category && (
          <span>
            {t("memory.category", { defaultValue: "Category" })}: {memory.category}
          </span>
        )}
      </div>
    </Card>
  );
}
