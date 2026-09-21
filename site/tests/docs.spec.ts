import { test, expect, type Page } from '@playwright/test'

const PAGE = '/docs/reference/cli'

const aside = (page: Page) => page.locator('aside[data-sticky]')
const header = (page: Page) => page.locator('header')

// Anything pinned has to clear the header, whether the visitor scrolled there or landed there.
async function below(page: Page, target: string) {
  const top = (await page.locator(target).boundingBox())!
  const bar = (await header(page).boundingBox())!

  expect(top.y, `${target} sits under the header`).toBeGreaterThanOrEqual(bar.y + bar.height)
}

test('marks the open page in the aside', async ({ page }) => {
  await page.goto(PAGE)

  await expect(aside(page).locator('[aria-current="page"]')).toHaveText('Command line interface')
  await expect(aside(page).locator('[aria-current="page"]')).toHaveCount(1)
})

test('leaves the aside pinned below the header while scrolling', async ({ page }) => {
  await page.goto(PAGE)
  await expect(aside(page)).toBeInViewport()

  await page.mouse.wheel(0, 1200)
  await page.waitForTimeout(300)

  await expect(aside(page)).toBeInViewport()
  await below(page, 'aside[data-sticky]')
})

// Landing on an anchor scrolls before any script runs, which is where the placement used to break.
for (const hash of ['#install', '#edge-build-pack-the-app']) {
  test(`keeps the page whole when it opens at ${hash}`, async ({ page }) => {
    await page.goto(PAGE + hash)

    await expect(aside(page)).toBeInViewport()
    await below(page, 'aside[data-sticky]')
    await below(page, hash)
  })
}

test.describe('the docs nav on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true })

  const sheet = (page: Page) => page.locator('#docs-nav')

  test('swaps the aside for a sheet naming the open page', async ({ page }) => {
    await page.goto(PAGE)

    await expect(aside(page)).toBeHidden()
    await expect(page.getByRole('button', { name: 'Command line interface' })).toBeVisible()
    await expect(sheet(page)).toBeHidden()
  })

  test('raises the sheet and walks to another page', async ({ page }) => {
    await page.goto(PAGE)

    await page.getByRole('button', { name: 'Command line interface' }).tap()
    await expect(sheet(page)).toBeVisible()

    await sheet(page).getByRole('link', { name: 'Built-in functions', exact: true }).tap()

    await expect(sheet(page)).toBeHidden()
    await expect(page).toHaveURL('/docs/reference/builtins')
  })
})
