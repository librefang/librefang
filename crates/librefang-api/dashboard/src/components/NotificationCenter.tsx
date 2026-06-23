import { useState, useEffect, useMemo, useCallback, useId, useRef } from "react";
import { Bell, Check, X, ExternalLink } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useUIStore } from "../lib/store";
import { useNavigate } from "@tanstack/react-router";
import { useApprovalCount, useApprovals, useTotpStatus } from "../lib/queries/approvals";
import { usePendingSkillCandidates } from "../lib/queries/skills";
import { useApproveApproval, useRejectApproval } from "../lib/mutations/approvals";

const POLL_INTERVAL_MS = 5_000;
const MAX_VISIBLE_ITEMS = 10;
const MAX_BADGE_COUNT = 99;

// Roving-tabindex menu items are addressed via this data attribute so the
// key handler can enumerate them with a single querySelectorAll regardless
// of how the menu body is composed.
const MENUITEM_ATTR = "data-notif-menuitem";

export function NotificationCenter() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  // -1 = "no item focused yet" (e.g. just opened by mouse click); >=0 = the
  // currently-focused menuitem index. Drives roving tabindex on render and
  // is the source of truth for ArrowUp/Down navigation.
  const [activeIndex, setActiveIndex] = useState(-1);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  // Stable ids so the trigger's aria-controls and the menu's
  // aria-labelledby point at each other deterministically across renders.
  const triggerId = useId();
  const menuId = useId();
  // Tracks whether the most recent open() came from a keyboard activation
  // (Enter / Space / ArrowDown on the trigger). If so, we auto-focus the
  // first item; mouse clicks leave focus on the trigger per WAI-ARIA APG.
  const openedByKeyboardRef = useRef(false);
  // When the menu was opened via ArrowUp on the trigger, focus the LAST
  // menuitem instead of the first (WAI-ARIA APG optional Up Arrow behavior).
  // Read once by the open effect, which is the only place that can move focus
  // after the menu has actually committed to the DOM.
  const openToLastRef = useRef(false);

  // Close the menu when focus leaves both the trigger and the menu — this
  // covers Tab-out, click-out (mousedown on a sibling moves activeElement),
  // and screenreader virtual-cursor focus moves. Also re-snap focus to the
  // trigger on Escape so keyboard users don't get stranded on document.body.
  useEffect(() => {
    if (!open) return;
    const onDocFocus = (e: FocusEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (menuRef.current?.contains(target)) return;
      if (triggerRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("focusin", onDocFocus);
    return () => document.removeEventListener("focusin", onDocFocus);
  }, [open]);

  const addToast = useUIStore((s) => s.addToast);
  const navigate = useNavigate();

  const countQuery = useApprovalCount({ refetchInterval: POLL_INTERVAL_MS });
  const listQuery = useApprovals({ enabled: open });
  const totpQuery = useTotpStatus({ enabled: open });
  const approveMutation = useApproveApproval();
  const rejectMutation = useRejectApproval();
  // Skill workshop pending queue (#3328) — surfaced alongside tool
  // approvals so the operator has a single notification surface for
  // anything that needs human attention. Polled at the same cadence
  // as approvals; the workshop is opt-in and most agents won't ever
  // populate this list.
  const pendingSkillsQuery = usePendingSkillCandidates(undefined, {
    refetchInterval: POLL_INTERVAL_MS,
  });
  const pendingSkillsCount = pendingSkillsQuery.data?.length ?? 0;

  const totpEnforced = totpQuery.data?.enforced ?? false;

  const approvalCount = countQuery.data ?? 0;
  const pendingCount = approvalCount + pendingSkillsCount;
  const pendingItems = useMemo(
    () => (listQuery.data ?? []).filter((a) => !a.status || a.status === "pending"),
    [listQuery.data]
  );

  const handleAction = useCallback(async (id: string, action: "approve" | "reject") => {
    // When TOTP is enforced, redirect to Approvals page for approve
    if (action === "approve" && totpEnforced) {
      setOpen(false);
      navigate({ to: "/approvals" });
      addToast(t("approvals.totpRequired", "TOTP code required. Use the Approvals page."), "info");
      return;
    }
    try {
      if (action === "approve") await approveMutation.mutateAsync({ id });
      else await rejectMutation.mutateAsync(id);
      addToast(
        t(`approvals.${action === "approve" ? "approvedToast" : "rejectedToast"}`),
        "success"
      );
    } catch {
      addToast(t("approvals.action_failed", "Action failed"), "error");
    }
  }, [totpEnforced, approveMutation, rejectMutation, addToast, navigate, t]);

  const goToAgent = useCallback((agentId: string) => {
    setOpen(false);
    navigate({ to: "/chat", search: { agentId } });
  }, [navigate]);

  // Centralised close: clears keyboard state and returns focus to the
  // trigger. Use this for Escape, Tab-out, and post-action close so the
  // bell button is always the next stop in the tab order.
  const closeAndReturnFocus = useCallback(() => {
    setOpen(false);
    setActiveIndex(-1);
    openedByKeyboardRef.current = false;
    openToLastRef.current = false;
    // requestAnimationFrame because the menu unmounts on `open=false`; if we
    // call .focus() synchronously the trigger may not yet own the layout
    // pass that lets it scroll into view correctly.
    requestAnimationFrame(() => triggerRef.current?.focus());
  }, []);

  // Enumerate currently-rendered menuitems. Done lazily on every key press
  // so we don't have to invalidate a memo when the underlying lists change.
  const getMenuItems = useCallback((): HTMLElement[] => {
    const root = menuRef.current;
    if (!root) return [];
    return Array.from(
      root.querySelectorAll<HTMLElement>(`[${MENUITEM_ATTR}]`),
    );
  }, []);

  // After the menu opens via keyboard, move focus to a menuitem. Runs
  // whenever `open` flips and items become available; gated on the ref so we
  // don't steal focus from a mouse interaction. `openToLastRef` selects the
  // last item (ArrowUp on trigger) vs the first (ArrowDown / Enter / Space).
  useEffect(() => {
    if (!open) return;
    if (!openedByKeyboardRef.current) return;
    const items = getMenuItems();
    if (items.length === 0) return;
    const target = openToLastRef.current ? items.length - 1 : 0;
    setActiveIndex(target);
    items[target].focus();
    openedByKeyboardRef.current = false;
    openToLastRef.current = false;
  }, [open, getMenuItems, pendingItems, pendingSkillsCount]);

  // Keep DOM focus in sync with activeIndex (e.g. ArrowDown updates the
  // index, this effect actually moves focus). Skip when no item is active.
  useEffect(() => {
    if (!open || activeIndex < 0) return;
    const items = getMenuItems();
    const el = items[activeIndex];
    if (el && document.activeElement !== el) {
      el.focus();
    }
  }, [activeIndex, open, getMenuItems]);

  const onTriggerKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLButtonElement>) => {
      // WAI-ARIA Menu Button: Enter/Space toggles, ArrowDown opens + focuses
      // first item, ArrowUp opens + focuses last item. Escape on a closed
      // menu is a no-op (handled by menu container when open).
      if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        openedByKeyboardRef.current = true;
        setOpen(true);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        openedByKeyboardRef.current = true;
        openToLastRef.current = true;
        setOpen(true);
      }
    },
    [],
  );

  const onMenuKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      // Only act on key events targeting a menuitem (or the menu chrome
      // itself). Buttons inside menuitems (approve / reject) bubble here
      // too; we let Enter/Space on them through so they activate normally.
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeAndReturnFocus();
        return;
      }
      if (e.key === "Tab") {
        // Per APG, Tab closes the menu and the browser then moves focus
        // forward (or back on Shift+Tab) from the trigger naturally.
        e.preventDefault();
        closeAndReturnFocus();
        return;
      }
      const items = getMenuItems();
      if (items.length === 0) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((cur) => (cur + 1 + items.length) % items.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((cur) => (cur <= 0 ? items.length - 1 : cur - 1));
        return;
      }
      if (e.key === "Home") {
        e.preventDefault();
        setActiveIndex(0);
        return;
      }
      if (e.key === "End") {
        e.preventDefault();
        setActiveIndex(items.length - 1);
        return;
      }
    },
    [closeAndReturnFocus, getMenuItems],
  );

  const onTriggerClick = useCallback(() => {
    if (open) {
      closeAndReturnFocus();
    } else {
      openedByKeyboardRef.current = false;
      setOpen(true);
    }
  }, [open, closeAndReturnFocus]);

  return (
    <div className="relative">
      <button
        ref={triggerRef}
        id={triggerId}
        onClick={onTriggerClick}
        onKeyDown={onTriggerKeyDown}
        className="relative flex h-9 w-9 items-center justify-center rounded-xl text-text-dim hover:text-brand hover:bg-surface-hover transition-colors duration-200"
        aria-label={pendingCount > 0 ? `${t("approvals.notifications", "Notifications")} (${pendingCount})` : t("approvals.notifications", "Notifications")}
        aria-expanded={open}
        aria-haspopup="menu"
        aria-controls={menuId}
      >
        <Bell className="h-4 w-4" />
        {countQuery.isError ? (
          <span className="absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full bg-error/60 ring-2 ring-surface" title={t("approvals.connection_error", "Connection error")} />
        ) : pendingCount > 0 ? (
          <span className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-error px-1 text-[10px] font-bold text-white">
            {pendingCount > MAX_BADGE_COUNT ? `${MAX_BADGE_COUNT}+` : pendingCount}
          </span>
        ) : null}
      </button>

      {open && (
        <>
          <div
            className="fixed inset-0 z-[90]"
            onClick={() => {
              setOpen(false);
              setActiveIndex(-1);
              openedByKeyboardRef.current = false;
              openToLastRef.current = false;
            }}
          />
          {/* position:fixed — the topbar's parent flex column has
              overflow-hidden, which would otherwise clip an absolute panel
              that extends below the topbar. Anchor to the topbar bottom
              (h-12 = 48px) + a 6px gap, right-aligned to the topbar padding. */}
          <div
            ref={menuRef}
            id={menuId}
            role="menu"
            aria-labelledby={triggerId}
            tabIndex={-1}
            onKeyDown={onMenuKeyDown}
            className="fixed top-[54px] right-3 sm:right-4 z-[100] w-[min(calc(100vw-1.5rem),24rem)] rounded-xl border border-border-subtle bg-surface shadow-xl"
          >
            {/* Indices are computed inline so roving tabindex stays in sync
                with whatever subset of menuitems is rendered this pass
                (header "View all" link, per-approval rows, skill-candidate
                shortcut). `menuItemIndex` is mutated as we go — keep render
                order stable. */}
            {(() => {
              let menuItemIndex = 0;
              const rowItems = pendingItems.slice(0, MAX_VISIBLE_ITEMS);
              return (
                <>
                  <div className="px-4 py-3 border-b border-border-subtle flex items-center justify-between">
                    <h3 className="text-sm font-bold text-text-main">
                      {t("approvals.pending_review", "Pending Review")}
                    </h3>
                    {pendingItems.length > 0 && (() => {
                      const idx = menuItemIndex++;
                      return (
                        <button
                          {...{ [MENUITEM_ATTR]: idx }}
                          role="menuitem"
                          tabIndex={idx === activeIndex ? 0 : -1}
                          onClick={() => {
                            setOpen(false);
                            setActiveIndex(-1);
                            navigate({ to: "/approvals" });
                          }}
                          className="text-xs text-brand hover:underline"
                        >
                          {t("common.viewAll", "View all")}
                        </button>
                      );
                    })()}
                  </div>
                  <div className="max-h-96 overflow-y-auto">
                    {pendingItems.length === 0 && pendingSkillsCount === 0 ? (
                      <div className="px-4 py-6 text-center text-sm text-text-dim">
                        {t("approvals.queue_clear_desc", "All clear")}
                      </div>
                    ) : (
                      rowItems.map((item) => {
                        const idx = menuItemIndex++;
                        return (
                          <div
                            key={item.id}
                            {...{ [MENUITEM_ATTR]: idx }}
                            role="menuitem"
                            tabIndex={idx === activeIndex ? 0 : -1}
                            // Enter / Space on the row navigates to the full
                            // Approvals page where the user has the complete
                            // approve / reject / modify-and-retry surface.
                            onClick={() => {
                              setOpen(false);
                              setActiveIndex(-1);
                              navigate({ to: "/approvals" });
                            }}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                setOpen(false);
                                setActiveIndex(-1);
                                navigate({ to: "/approvals" });
                              }
                            }}
                            className="px-4 py-3 border-b last:border-0 border-border-subtle hover:bg-surface-hover focus:bg-surface-hover focus:outline-none transition-colors cursor-pointer"
                          >
                            <div className="flex items-start justify-between gap-2">
                              <div className="min-w-0 flex-1">
                                <div className="flex items-center gap-1.5">
                                  <p className="text-sm font-medium text-text-main truncate">
                                    {item.tool_name}
                                  </p>
                                  {item.risk_level && (
                                    <span className={`text-[10px] px-1.5 py-0.5 rounded font-bold uppercase ${
                                      item.risk_level === "critical" ? "bg-error/10 text-error" :
                                      item.risk_level === "high" ? "bg-warning/10 text-warning" :
                                      "bg-surface-hover text-text-dim"
                                    }`}>
                                      {item.risk_level}
                                    </span>
                                  )}
                                </div>
                                {item.agent_id && (
                                  <button
                                    // Sub-control inside a menuitem; excluded
                                    // from the tab order so the row remains
                                    // the single focus stop (roving tabindex).
                                    tabIndex={-1}
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      goToAgent(item.agent_id!);
                                    }}
                                    className="flex items-center gap-1 text-xs text-brand hover:underline mt-0.5"
                                    title={t("approvals.goToAgent", "Open agent chat")}
                                  >
                                    <span className="truncate">{item.agent_name ?? item.agent_id}</span>
                                    <ExternalLink className="w-3 h-3 shrink-0" />
                                  </button>
                                )}
                                {(item.action_summary || item.description) && (
                                  <p className="text-xs text-text-dim mt-1 line-clamp-2">
                                    {item.action_summary || item.description}
                                  </p>
                                )}
                              </div>
                              <div className="flex gap-1 shrink-0">
                                <button
                                  tabIndex={-1}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    handleAction(item.id, "approve");
                                  }}
                                  className="p-1 rounded hover:bg-success/10 text-success transition-colors"
                                  title={t("approvals.approve")}
                                  aria-label={t("approvals.approve")}
                                >
                                  <Check className="w-4 h-4" />
                                </button>
                                <button
                                  tabIndex={-1}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    handleAction(item.id, "reject");
                                  }}
                                  className="p-1 rounded hover:bg-error/10 text-error transition-colors"
                                  title={t("approvals.reject")}
                                  aria-label={t("approvals.reject")}
                                >
                                  <X className="w-4 h-4" />
                                </button>
                              </div>
                            </div>
                          </div>
                        );
                      })
                    )}
                    {pendingSkillsCount > 0 && (() => {
                      const idx = menuItemIndex++;
                      return (
                        <button
                          {...{ [MENUITEM_ATTR]: idx }}
                          role="menuitem"
                          tabIndex={idx === activeIndex ? 0 : -1}
                          onClick={() => {
                            setOpen(false);
                            setActiveIndex(-1);
                            // `search: { tab: "pending" }` deep-links into the
                            // SkillsPage `Pending` tab via the optional `?tab=`
                            // search param the page reads on mount. Cast to
                            // `never` because the route doesn't declare a
                            // search schema (the parameter is optional and
                            // only consumed by SkillsPage itself).
                            navigate({
                              to: "/skills",
                              search: { tab: "pending" } as never,
                            });
                          }}
                          className={`flex w-full items-center justify-between gap-2 ${pendingItems.length > 0 ? "border-t border-border-subtle" : ""} px-4 py-3 text-left text-sm hover:bg-surface-hover focus:bg-surface-hover focus:outline-none transition-colors`}
                        >
                          <span className="text-text-main">
                            {t("approvals.skill_candidates_pending", {
                              defaultValue: "{{count}} skill candidates pending review",
                              count: pendingSkillsCount,
                            })}
                          </span>
                          <ExternalLink className="w-3 h-3 shrink-0 text-text-dim" />
                        </button>
                      );
                    })()}
                  </div>
                </>
              );
            })()}
          </div>
        </>
      )}
    </div>
  );
}
