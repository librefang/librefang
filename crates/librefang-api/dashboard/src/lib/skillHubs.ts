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
  /** Public web page for one skill on this hub, or `null` when the hub has
   *  no page we can address. FangHub ships no public web UI, and SkillHub is
   *  self-hosted — its origin is only known when `VITE_SKILLHUB_REGISTRY_URL`
   *  is configured at build time. Returning `null` is what keeps the UI from
   *  offering a link that lands on an unrelated site. */
  skillUrl: (slug: string) => string | null;
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
// Web origin of the configured SkillHub deployment. `skillHubRegistryUrl` is
// an API base (`…/api/v1`), so the browsable page hangs off its origin.
const skillHubWebOrigin = skillHubRegistryUrl
  ? new URL(skillHubRegistryUrl).origin
  : undefined;

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
    skillUrl: () => null,
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
    skillUrl: (slug) =>
      skillHubWebOrigin
        ? `${skillHubWebOrigin}/skills/${encodeURIComponent(slug)}`
        : null,
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
    skillUrl: (slug) => `https://clawhub.ai/skills/${encodeURIComponent(slug)}`,
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
    // The mirror the backend actually talks to (`CLAWHUB_CN_BASE_URL` in
    // `routes/skills/mod.rs`) is `mirror-cn.clawhub.com`, which fronts the
    // browsable pages too.
    skillUrl: (slug) =>
      `https://mirror-cn.clawhub.com/skills/${encodeURIComponent(slug)}`,
  },
} as const satisfies SkillHubIndex;

export const SKILL_HUBS: readonly SkillHub[] = Object.values(HUB_INDEX);
const HUB_LOOKUP: ReadonlyMap<string, SkillHub> = new Map(
  SKILL_HUBS.map((hub) => [hub.id, hub]),
);

export function getSkillHub(id: string): SkillHub | undefined {
  return HUB_LOOKUP.get(id);
}

/** Public marketplace page for `slug` on hub `id`, or `null` when the hub is
 *  unknown, the slug is empty, or the hub exposes no addressable page. */
export function skillHubUrl(id: string | undefined, slug: string | undefined): string | null {
  if (!id || !slug?.trim()) return null;
  return getSkillHub(id)?.skillUrl(slug.trim()) ?? null;
}
