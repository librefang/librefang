/**
 * Federated skill hub configuration — single source of truth for the
 * Skills page's hub metadata (colors, glyphs, CLI templates) and the
 * shared UI bits (HubBadge, HubSourceBar, etc. — see
 * `components/SkillHubBar.tsx`).
 *
 * Backend already exposes per-hub endpoints:
 *   GET /api/skills            (installed)
 *   GET /api/skills/registry   (FangHub)
 *   GET /clawhub/{search,browse,install}
 *   GET /clawhub-cn/{search,browse,install}
 *   GET /skillhub/{search,browse,install}
 *
 * The dashboard's `lib/queries/skills.ts` and `lib/mutations/skills.ts`
 * already expose query/mutation hooks for each. This config layer
 * unifies how the UI presents them.
 */

export type SkillHubId = "fanghub" | "skillhub" | "clawhub" | "clawhub-cn";

export type SkillHub = {
  id: SkillHubId;
  /** Display name in the source bar / badges. */
  name: string;
  /** One-character glyph used as the hub icon. */
  glyph: string;
  /** Hex color the hub renders in. */
  color: string;
  /** Public domain the hub serves from. Shown in detail copy. */
  domain: string;
  /** One-line description for the hub overview tile. */
  desc: string;
  /** CLI install command template. `slug` is the registry slug. */
  cli: (slug: string) => string;
};

export const SKILL_HUBS: readonly SkillHub[] = [
  {
    id: "fanghub",
    name: "FangHub",
    glyph: "🪝",
    color: "#FF6A3D",
    domain: "fanghub.librefang.ai",
    desc:
      "Official LibreFang registry — curated hands, agents, MCP, providers, plugins.",
    cli: (slug) => `librefang skill install ${slug}`,
  },
  {
    id: "skillhub",
    name: "SkillHub",
    glyph: "🛡",
    color: "#a78bfa",
    domain: "skillhub.your-co.com",
    desc:
      "Self-hosted enterprise skill registry — private namespaces behind your firewall.",
    cli: (slug) =>
      `CLAWHUB_REGISTRY=https://skillhub.your-co.com clawhub install ${slug}`,
  },
  {
    id: "clawhub",
    name: "ClawHub",
    glyph: "🦞",
    color: "#fb923c",
    domain: "clawhub.ai",
    desc:
      "OpenClaw public registry — thousands of community skills, vector search.",
    cli: (slug) => `clawhub install ${slug}`,
  },
  {
    id: "clawhub-cn",
    name: "ClawHub-CN",
    glyph: "🇨🇳",
    color: "#f87171",
    domain: "clawhub.cn",
    desc:
      "ClawHub China mirror — accelerated access, CN-native skills.",
    cli: (slug) =>
      `CLAWHUB_REGISTRY=https://clawhub.cn clawhub install ${slug}`,
  },
] as const;

const HUB_INDEX: Readonly<Record<SkillHubId, SkillHub>> = Object.fromEntries(
  SKILL_HUBS.map((h) => [h.id, h]),
) as Record<SkillHubId, SkillHub>;

export function getSkillHub(id: SkillHubId): SkillHub {
  return HUB_INDEX[id];
}
