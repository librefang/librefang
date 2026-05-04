# Dashboard — Agent Instructions

React 19 + TanStack Router v1 + TanStack Query v5 SPA. Entry: `src/main.tsx`. Pages in `src/pages/`.

## Data layer — mandatory rules

All data access from pages/components goes through the shared hooks layer. Do NOT call `fetch()` or `api.*` directly inside a page or component file.

### Layout

```
src/lib/
  http/
    client.ts     # thin wrapper over src/api.ts + typed re-exports
    errors.ts     # ApiError class used by the wrapper
  queries/
    keys.ts       # all query-key factories — edit here when adding a domain
    keys.test.ts  # smoke tests — add cases when you add a factory
    <domain>.ts   # queryOptions + useXxx hooks per domain
  mutations/
    <domain>.ts   # useXxx mutation hooks with invalidation
```

Domain files today: `agents`, `analytics`, `approvals`, `channels`, `config`, `goals`, `hands`, `mcp`, `media`, `memory`, `models`, `network`, `overview`, `plugins`, `providers`, `runtime`, `schedules`, `sessions`, `skills`, `workflows`.

### Adding a new endpoint

1. Add the raw call in `src/api.ts` (or re-export via `src/lib/http/client.ts`).
2. If it is a new domain, add a factory in `src/lib/queries/keys.ts` following the hierarchical pattern:
   ```ts
   export const fooKeys = {
     all: ["foo"] as const,
     lists: () => [...fooKeys.all, "list"] as const,
     list: (filters: FooFilters = {}) => [...fooKeys.lists(), filters] as const,
     details: () => [...fooKeys.all, "detail"] as const,
     detail: (id: string) => [...fooKeys.details(), id] as const,
   };
   ```
   Every sub-key MUST be anchored with `...fooKeys.all` so broad invalidation works.
3. Add the query in `src/lib/queries/<domain>.ts`:
   ```ts
   export const fooQueryOptions = (filters?: FooFilters) =>
     queryOptions({
       queryKey: fooKeys.list(filters ?? {}),
       queryFn: () => listFoo(filters),
       staleTime: 30_000,
     });
   export function useFoo(filters?: FooFilters) {
     return useQuery(fooQueryOptions(filters));
   }
   ```
4. Add mutations in `src/lib/mutations/<domain>.ts`. **Every write MUST invalidate**, and invalidation MUST live inside the hook. (Call sites MAY additionally attach per-call `onSuccess` / `onError` for UI feedback — see the Conventions section.) **Prefer the narrowest matching keys. Use `fooKeys.all` only when the mutation truly dirties every sub-key in the domain.**

   Pick the narrowest set that covers what actually changed:
   - `fooKeys.detail(id)` + `fooKeys.lists()` — per-id update where the list projection also changes (patch, rename, status flag surfaced in the list row). This is the **default** template.
   - `fooKeys.lists()` — list-shape change with no existing detail to refresh (create, delete, reorder).
   - `fooKeys.detail(id)` or a nested sub-key like `fooKeys.experiments(id)` — change is genuinely scoped to one detail or one nested collection and the list projection is unaffected.
   - `fooKeys.all` — bulk import / cache reset / cross-cutting schema migration. Not the default.

   Fan-out trade-off: invalidating `fooKeys.all` while N items are cached refetches the list plus every cached sub-key (`detail(id)`, plus any nested keys like `sessions(id)`, `experiments(id)`) for each of the N items. Use it only when that is the desired effect; otherwise prefer the narrower keys.

   ```ts
   // Default: per-id patch where the list projection also changes.
   // Matches shipped usePatchAgentConfig, experiment mutations, etc.
   export function useUpdateFoo() {
     const qc = useQueryClient();
     return useMutation({
       mutationFn: updateFoo,
       onSuccess: (_data, variables) => {
         qc.invalidateQueries({ queryKey: fooKeys.lists() });
         qc.invalidateQueries({ queryKey: fooKeys.detail(variables.id) });
       },
     });
   }

   // Lists-only: membership changed, no existing detail is stale.
   export function useCreateFoo() {
     const qc = useQueryClient();
     return useMutation({
       mutationFn: createFoo,
       onSuccess: () => qc.invalidateQueries({ queryKey: fooKeys.lists() }),
     });
   }

   // Bulk import / cache reset — NOT the default template.
   // Every cached Foo and its sub-keys are potentially stale.
   export function useImportFoos() {
     const qc = useQueryClient();
     return useMutation({
       mutationFn: importFoos,
       onSuccess: () => qc.invalidateQueries({ queryKey: fooKeys.all }),
     });
   }
   ```
5. Update `src/lib/queries/keys.test.ts` — at minimum add the new factory to the `all factories exist` list. Add anchoring/hierarchy tests for non-trivial factories.

### Consuming in pages

```tsx
import { useFoo } from "../lib/queries/foo";
import { useCreateFoo } from "../lib/mutations/foo";

function FooPage() {
  const { data, isLoading } = useFoo({ active: true });
  const createFoo = useCreateFoo();
  // ...
}
```

Never build a `queryKey` inline — always call the factory. Never subscribe to the same endpoint with a different key just to get a subset; use `select` on the shared `queryOptions`.

### Exceptions (not cached data)

Streaming / SSE, imperative fire-and-forget control channels (e.g. `src/components/TerminalTabs.tsx` terminal window lifecycle), and one-shot probes that must not be cached may call `fetch` directly. Keep these narrow and comment why.

## Build & verify

```bash
pnpm typecheck                # tsc --noEmit — must be green
pnpm test --run               # vitest — all tests pass
pnpm build                    # vite build — must succeed
```

Run all three after any change to `src/lib/queries/`, `src/lib/mutations/`, or `src/api.ts`. A passing typecheck alone is not enough — the key-factory tests catch anchoring regressions that the compiler does not.

## Conventions

- TypeScript strict. No `any` in new hooks; lean on types from `src/api.ts` or `openapi/generated.ts`.
- Hooks set sensible defaults in `queryOptions` (shared `staleTime` / `refetchInterval` so consumers without special needs inherit one policy). Accept an optional `options: { enabled?; staleTime?; refetchInterval? }` argument and pass it through to `useQuery` so call sites can override per-page needs — bell-icon polls fast but gated, bulk-management pages poll slowly, tabs gate by active-tab. See `useApprovals({ enabled: open })`, `useCommsEvents(50, { refetchInterval: 5_000 })`, `useModels({}, { enabled: isModelArg })`, `useAgentTemplates({ enabled })`, and `useApprovalCount({ refetchInterval: 5_000 })` for reference shapes. Every call-site override carries a short inline comment explaining why.
  ```ts
  type UseFooOptions = {
    enabled?: boolean;
    staleTime?: number;
    refetchInterval?: number | false;
  };
  export function useFoo(filters?: FooFilters, options: UseFooOptions = {}) {
    const { enabled, staleTime, refetchInterval } = options;
    return useQuery({
      ...fooQueryOptions(filters),
      enabled,
      staleTime,
      refetchInterval,
    });
  }
  ```
- Mutation invalidation lives in the hook — callers should never need to know which keys a mutation touches. Call sites MAY attach per-call `onSuccess` / `onError` handlers for UI feedback (toasts, modal dismissal, local state updates); that is orthogonal to invalidation and stays at the call site. See `MemoryPage` delete/cleanup and `ChannelsPage` configure/test for the pattern.
- Commit convention matches the root repo: `feat(dashboard/<area>): ...`, `refactor(dashboard/queries): ...`, `fix(dashboard/<area>): ...`. Never include a `Co-Authored-By` footer.

## BossFang Branding

This dashboard is part of the **BossFang** product. Upstream (LibreFang) visual
identity must never appear in committed code. These rules are absolute — they apply
to new components, refactors, and post-merge cleanups alike.

### Identity constants

| Element | Value | Never use |
|---|---|---|
| Product name | `BossFang` | `librefang`, `LibreFang` |
| Logo | `<img src="/boss-libre.png" alt="BossFang" ...>` | SVG fang glyph, sky-blue gradient box |
| Light brand color | `#E04E28` / `var(--brand-color)` | `#0284c7`, `#0ea5e9`, any sky-blue |
| Dark brand color | `#FF6A3D` / `var(--brand-color)` | `#38bdf8`, any sky-blue |
| Dark background | `#0B0F14` / `var(--bg-main)` | `#020617`, `rgba(2,6,23,...)` |

### CSS rules

- **Always** use `var(--brand-color)` for brand-colored elements, or the Tailwind
  utility classes `text-brand`, `bg-brand`, `border-brand` which resolve to that variable.
- **Never** hardcode `#0284c7`, `#38bdf8`, `#0ea5e9`, or any sky-blue hex/rgba in
  `style={{}}` props or CSS files.
- When upstream adds a new component with a hardcoded gradient like
  `linear-gradient(135deg,#38bdf8,#0ea5e9)`, replace the sky-blue values with the
  ember equivalents (`#FF6A3D` → `#E04E28`) before committing.

### Tailwind utility quick reference

```
text-brand          → brand-colored text (ember orange)
bg-brand            → brand-colored background
border-brand        → brand-colored border
hover:text-brand    → hover brand text
shadow-brand        → brand glow shadow
ring-brand          → focus ring in brand color
bg-brand/15         → 15% opacity brand background
border-brand/30     → 30% opacity brand border
```

### Logo usage

Sidebar brand block and mobile header must always render:
```tsx
<img
  src="/boss-libre.png"
  alt="BossFang"
  className="h-[26px] w-[26px] rounded-[7px] object-contain shrink-0"
/>
```
Never replace with an SVG icon, emoji, or gradient-box logo from upstream.

### After every upstream merge

Run the enforcement script from the repo root before committing:
```bash
python3 scripts/enforce-branding.py
```
It automatically replaces all sky-blue upstream tokens with ember equivalents across
the entire `dashboard/src/` tree. Review the diff afterward to catch any SVG-glyph
or gradient-box logo regressions that the script cannot handle automatically.
