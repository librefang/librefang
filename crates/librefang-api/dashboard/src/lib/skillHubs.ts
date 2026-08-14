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

const normalizeRegistryUrl = (raw: string | undefined): string | undefined => {
  if (!raw?.trim()) return undefined;
  try {
    const url = new URL(raw.trim());
    if (url.protocol !== "http:" && url.protocol !== "https:") return undefined;
    if (url.username || url.password || url.search || url.hash) return undefined;
    return url.href.replace(/\/+$/, "");
  } catch {
    return undefined;
  }
};

const shellQuote = (value: string): string =>
  `'${value.replace(/'/g, `'"'"'`)}'`;

const skillHubRegistryUrl = normalizeRegistryUrl(
  import.meta.env.VITE_SKILLHUB_REGISTRY_URL,
);
const skillHubDomain = skillHubRegistryUrl
  ? new URL(skillHubRegistryUrl).host
  : "deployment configured";
const skillHubCliRegistry = skillHubRegistryUrl
  ? shellQuote(skillHubRegistryUrl)
  : '"$SKILLHUB_REGISTRY_URL"';

type SkillHubIndex = {
  readonly [K in SkillHubId]: SkillHub & { readonly id: K };
};

const HUB_INDEX = {
  fanghub: {
    id: "fanghub",
    name: "FangHub",
    glyph: "🪝",
    color: "#38bdf8",
    domain: "fanghub.librefang.ai",
    desc:
      "Official LibreFang registry — curated hands, agents, MCP, providers, plugins.",
    cli: (slug) => `librefang skill install ${shellQuote(slug)}`,
  },
  skillhub: {
    id: "skillhub",
    name: "SkillHub",
    glyph: "🛡",
    color: "#a78bfa",
    domain: skillHubDomain,
    desc:
      "Self-hosted enterprise skill registry — private namespaces behind your firewall.",
    cli: (slug) =>
      `CLAWHUB_REGISTRY=${skillHubCliRegistry} clawhub install ${shellQuote(slug)}`,
  },
  clawhub: {
    id: "clawhub",
    name: "ClawHub",
    glyph: "🦞",
    color: "#fb923c",
    domain: "clawhub.ai",
    desc:
      "OpenClaw public registry — thousands of community skills, vector search.",
    cli: (slug) => `clawhub install ${shellQuote(slug)}`,
  },
  "clawhub-cn": {
    id: "clawhub-cn",
    name: "ClawHub-CN",
    glyph: "🇨🇳",
    color: "#f87171",
    domain: "clawhub.cn",
    desc:
      "ClawHub China mirror — accelerated access, CN-native skills.",
    cli: (slug) =>
      `CLAWHUB_REGISTRY=https://clawhub.cn clawhub install ${shellQuote(slug)}`,
  },
} as const satisfies SkillHubIndex;

export const SKILL_HUBS: readonly SkillHub[] = Object.values(HUB_INDEX);
const HUB_LOOKUP: ReadonlyMap<string, SkillHub> = new Map(
  SKILL_HUBS.map((hub) => [hub.id, hub]),
);

export function getSkillHub(id: string): SkillHub | undefined {
  return HUB_LOOKUP.get(id);
}
