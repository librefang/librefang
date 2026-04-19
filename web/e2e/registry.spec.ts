import { expect, test } from '@playwright/test'

test.describe('registry', () => {
  test('skills list loads cards', async ({ page }) => {
    await page.goto('/skills')
    await expect(page.locator('h1')).toContainText(/Skills/i)
    // At least one card renders. Wait for the registry fetch to complete.
    await expect(page.locator('a[href*="/skills/"]').first()).toBeVisible({ timeout: 15000 })
  })

  test('detail page shows manifest heading and install command', async ({ page }) => {
    await page.goto('/skills')
    const firstCard = page.locator('a[href*="/skills/"]').first()
    await firstCard.waitFor({ state: 'visible', timeout: 15000 })
    const href = await firstCard.getAttribute('href')
    expect(href).toBeTruthy()
    await page.goto(href!)
    // Header should show either the name or at least the id fallback.
    await expect(page.locator('h1')).toBeVisible()
    // Install command block (skills category always has one).
    await expect(page.getByText(/librefang\s+skill\s+install/)).toBeVisible({ timeout: 15000 })
  })

  test('Cmd+K opens search dialog', async ({ page }) => {
    await page.goto('/')
    // Simulate meta+K cross-platform: Playwright uses Meta on mac, Ctrl elsewhere.
    const mod = process.platform === 'darwin' ? 'Meta' : 'Control'
    await page.keyboard.press(`${mod}+KeyK`)
    // Dialog's placeholder text appears.
    await expect(page.getByPlaceholder(/search/i)).toBeVisible()
    // Esc closes.
    await page.keyboard.press('Escape')
    await expect(page.getByPlaceholder(/search/i)).not.toBeVisible()
  })
})
