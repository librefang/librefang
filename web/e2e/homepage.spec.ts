import { expect, test } from '@playwright/test'

test.describe('homepage', () => {
  test('renders hero and nav', async ({ page }) => {
    await page.goto('/')
    await expect(page).toHaveTitle(/LibreFang/)
    // Hero has the product name in an h1
    await expect(page.locator('h1').first()).toBeVisible()
    // Nav has the Features dropdown button
    await expect(page.getByRole('button', { name: /features/i })).toBeVisible()
  })

  test('Features dropdown reveals registry links', async ({ page }) => {
    await page.goto('/')
    await page.getByRole('button', { name: /features/i }).click()
    // Registry group has the 8 category links
    await expect(page.getByRole('link', { name: /^Skills$/ })).toBeVisible()
    await expect(page.getByRole('link', { name: /^Hands$/ })).toBeVisible()
  })

  test('language switch preserves path', async ({ page }) => {
    await page.goto('/skills')
    // Open lang switcher and pick Chinese.
    await page.getByLabel(/switch language/i).first().click()
    await page.getByRole('button', { name: '简体中文' }).click()
    await expect(page).toHaveURL(/\/zh\/skills/)
  })
})
