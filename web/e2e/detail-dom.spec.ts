import { expect, test } from '@playwright/test'

test.describe('detail page DOM', () => {
  test('TOML manifest renders highlighted spans', async ({ page }) => {
    await page.goto('/skills')
    const firstCard = page.locator('a[href*="/skills/"]').first()
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
    await page.goto('/skills')
    const firstCard = page.locator('a[href*="/skills/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    await firstCard.click()
    // Manifest heading has an anchor link that sets the hash on click.
    const anchor = page.locator('a[href="#manifest"]').first()
    await anchor.scrollIntoViewIfNeeded()
    await anchor.click({ force: true })
    await expect(page).toHaveURL(/#manifest$/)
  })

  test('related items section renders when data is available', async ({ page }) => {
    await page.goto('/skills')
    const firstCard = page.locator('a[href*="/skills/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    await firstCard.click()
    // Related section has its own id and heading. Use .first() because each
    // "More <cat>" block may also show in the search dialog's empty state.
    await expect(page.locator('#related h2').first()).toBeVisible()
  })
})
