// End-to-end coverage for the EveryAPI connect action on the Providers page.
//
// The unit tests in `src/pages/ProvidersPage.test.tsx` mock at the mutation-hook layer, so they prove the page calls `useConnectEveryApi` with the right arguments and nothing more.
// What they cannot see is the part that actually crosses a wire: the request paths, the JSON body shapes, and the order the two writes happen in.
// They also cannot see render-loop bugs, because a mocked hook returns a reference-stable object while the real `useMutation` returns a fresh one every render.
//
// The write ordering is load-bearing rather than incidental — `POST /api/providers/{id}/key` addresses a provider by id, so a key sent before the registry entry exists has nothing to attach to.
// And the registry body is a cross-language contract with the CLI: the daemon keys its catalog refresh off the provider id, and `models` is deliberately empty so `catalog_needs_initial_refresh` in `crates/librefang-api/src/everyapi_catalog.rs` becomes true and the daemon backfills the catalog itself.
//
// No daemon is involved.
// Playwright serves the dashboard from vite (see playwright.config.ts, port 4173) and every backend call is fulfilled by `page.route`, so this suite never contends with a real LibreFang on 4545.
// The flip side is that it verifies what the dashboard *sends*, not that the daemon accepts it.
//
// `.first()` throughout: the drawer host mounts at both the desktop and mobile breakpoints, so every control inside it resolves to two nodes with only one visible.
// Without it Playwright's strict mode rejects the locator outright.

import { expect, test, type Page } from "@playwright/test";

/** Providers deliberately without an `everyapi` entry — the state the connect action exists for. */
const PROVIDERS_WITHOUT_EVERYAPI = [
  {
    id: "openai",
    display_name: "OpenAI",
    auth_status: "validated_key",
    reachable: true,
    model_count: 12,
    key_required: true,
    base_url: "https://api.openai.com/v1",
  },
  {
    id: "groq",
    display_name: "Groq",
    auth_status: "missing",
    reachable: false,
    model_count: 0,
    key_required: true,
    base_url: "https://api.groq.com/openai/v1",
  },
];

type CapturedWrite = { path: string; method: string; body: unknown };

/** Stub the provider list and capture both writes the connect flow performs. */
async function stubProviderRoutes(page: Page, writes: CapturedWrite[]): Promise<void> {
  await page.route("**/api/providers", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(PROVIDERS_WITHOUT_EVERYAPI),
    });
  });

  for (const pattern of ["**/api/registry/content/provider", "**/api/providers/everyapi/key"]) {
    await page.route(pattern, async (route) => {
      writes.push({
        path: new URL(route.request().url()).pathname,
        method: route.request().method(),
        body: route.request().postDataJSON(),
      });
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true }),
      });
    });
  }
}

/** Navigate to Providers and open the EveryAPI connect drawer. */
async function openConnectDrawer(page: Page): Promise<void> {
  await page.goto("/");
  await page.getByRole("link", { name: "Providers" }).click();
  await page.getByRole("button", { name: "Add Provider" }).first().click();
  await page.getByRole("button", { name: "Connect EveryAPI gateway" }).first().click();
}

test("connecting EveryAPI writes the registry entry before the key", async ({ page }) => {
  const writes: CapturedWrite[] = [];
  await stubProviderRoutes(page, writes);
  await openConnectDrawer(page);

  // Leaving the gateway field empty on purpose: the default belongs to the mutation hook, and the assertion below pins that the page does not substitute one of its own.
  await page.getByPlaceholder("EVERYAPI_API_KEY").first().fill("relay-e2e-key");
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  await expect.poll(() => writes.length, { timeout: 10_000 }).toBe(2);

  const [registryWrite, keyWrite] = writes;

  // Order: the entry must exist before a key can be attached to it.
  expect(registryWrite.path).toBe("/api/registry/content/provider");
  expect(keyWrite.path).toBe("/api/providers/everyapi/key");

  expect(registryWrite.method).toBe("POST");
  expect(keyWrite.method).toBe("POST");

  // The registry body is the contract with the daemon.
  // `models: []` is what arms the daemon's initial catalog refresh, so an accidental change here silently leaves the provider with no models forever rather than failing loudly.
  expect(registryWrite.body).toEqual({
    id: "everyapi",
    display_name: "EveryAPI",
    api_key_env: "EVERYAPI_API_KEY",
    base_url: "https://api.everyapi.ai/v1",
    key_required: true,
    models: [],
  });

  expect(keyWrite.body).toEqual({ key: "relay-e2e-key" });
});

test("a custom gateway URL replaces the default in the registry write", async ({ page }) => {
  const writes: CapturedWrite[] = [];
  await stubProviderRoutes(page, writes);
  await openConnectDrawer(page);

  await page.getByPlaceholder("EVERYAPI_API_KEY").first().fill("relay-e2e-key");
  // Surrounding whitespace is what a paste out of a browser address bar looks like; the hook trims it and otherwise stores the typed value verbatim, because a self-hosted gateway mounted at a different path is a legitimate configuration.
  await page
    .getByPlaceholder("https://api.everyapi.ai")
    .first()
    .fill("  https://gw.internal:8443  ");
  await page.getByRole("button", { name: "Connect", exact: true }).first().click();

  await expect.poll(() => writes.length, { timeout: 10_000 }).toBe(2);
  expect(writes[0].body).toMatchObject({
    id: "everyapi",
    base_url: "https://gw.internal:8443",
  });
});
