import { expect, test } from '@playwright/test'

// Deterministic TOML payload fed to the page so the .toml-highlight renderer
// is exercised without the tests depending on stats.librefang.ai (its
// /api/registry/raw endpoint isn't live yet) or GitHub raw (rate-limited on
// CI). Both upstream URLs are intercepted — see fetchRegistryRaw.
const FIXTURE_TOML = `# Fixture manifest used by detail-dom e2e tests.
id = "fixture-hand"
name = "Fixture Hand"
description = "Deterministic manifest for Playwright"

[metadata]
category = "test"
version = "0.0.1"
`

test.describe('detail page DOM', () => {
  test.beforeEach(async ({ page }) => {
    await page.route('**/stats.librefang.ai/api/registry/raw**', route =>
      route.fulfill({ status: 200, contentType: 'text/plain', body: FIXTURE_TOML })
    )
    await page.route('**/raw.githubusercontent.com/librefang/librefang-registry/**', route =>
      route.fulfill({ status: 200, contentType: 'text/plain', body: FIXTURE_TOML })
    )
  })

  test('TOML manifest renders highlighted spans', async ({ page }) => {
    // Use /hands because skill manifests ship as SKILL.md (YAML frontmatter),
    // not TOML — only TOML-backed categories exercise the .toml-highlight
    // renderer (hands, agents, plugins, channels, providers, etc.).
    await page.goto('/hands')
    const firstCard = page.locator('a[href*="/hands/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    const href = await firstCard.getAttribute('href')
    await page.goto(href!)
    // Wait for manifest block to hydrate.
    await page.locator('.toml-highlight').waitFor({ state: 'visible', timeout: 15000 })
    // At least one header, key, and string token should be emitted by the
    // custom highlighter.
    await expect(page.locator('.toml-highlight .tk-header').first()).toBeVisible()
    await expect(page.locator('.toml-highlight .tk-key').first()).toBeVisible()
    await expect(page.locator('.toml-highlight .tk-str').first()).toBeVisible()
  })

  test('anchor copy-link hashes the URL', async ({ page }) => {
    await page.goto('/hands')
    const firstCard = page.locator('a[href*="/hands/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    await firstCard.click()
    // Manifest heading has an anchor link that sets the hash on click.
    const anchor = page.locator('a[href="#manifest"]').first()
    await anchor.scrollIntoViewIfNeeded()
    await anchor.click({ force: true })
    await expect(page).toHaveURL(/#manifest$/)
  })

  test('related items section renders when data is available', async ({ page }) => {
    await page.goto('/hands')
    const firstCard = page.locator('a[href*="/hands/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    await firstCard.click()
    // Related section has its own id and heading. Use .first() because each
    // "More <cat>" block may also show in the search dialog's empty state.
    await expect(page.locator('#related h2').first()).toBeVisible()
  })
})
