import { expect, test } from '@playwright/test'

test.describe('i18n', () => {
  test('zh homepage renders with Chinese nav', async ({ page }) => {
    await page.goto('/zh/')
    // Hero title should render in Chinese. Exact text is too brittle —
    // assert html lang + that the features dropdown has the localized label.
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh')
    // Features dropdown is "功能" in zh.
    await expect(page.getByRole('button', { name: '功能' })).toBeVisible()
  })

  test('zh skills list shows 技能 heading', async ({ page }) => {
    await page.goto('/zh/skills')
    await expect(page.locator('h1')).toContainText(/技能/)
  })

  test('hreflang links exist for all locales', async ({ page }) => {
    await page.goto('/')
    const langs = ['en', 'zh', 'zh-TW', 'ja', 'ko', 'de', 'es']
    for (const lang of langs) {
      await expect(page.locator(`link[hreflang="${lang}"]`)).toHaveCount(1)
    }
    await expect(page.locator('link[hreflang="x-default"]')).toHaveCount(1)
  })
})
