import { test, expect, type Page } from '@playwright/test'

const PAGE = '/package/json'

const scope = (page: Page) => page.locator('p', { hasText: /^Runs/ })

test('points the docs aside at the package it belongs to', async ({ page }) => {
  await page.goto(PAGE)

  const links = page.locator('aside[data-sticky] a')
  const count = await links.count()

  expect(count).toBeGreaterThan(0)
  for (let at = 0; at < count; at++) await expect(links.nth(at)).toHaveAttribute('href', /^\/package\/json\//)
})

test('names every host an open package reaches and says when there is only one', async ({ page }) => {
  await page.goto(PAGE)
  await expect(scope(page)).toHaveText('Runs in the CLI, a browser and an actor pool.')

  await page.goto('/package/dom')
  await expect(scope(page)).toHaveText('Runs only in a browser.')
})

// The sentence rode an ml-auto, so a band of widths left it wrapped onto its own line and still pushed right.
test('keeps the scope sentence beside the digest or below it, never adrift', async ({ page }) => {
  await page.goto(PAGE)

  for (const width of [1440, 1100, 900, 800, 700, 600, 480, 390]) {
    await page.setViewportSize({ width, height: 900 })

    const seen = await scope(page).evaluate((el) => {
      const digest = el.previousElementSibling!.getBoundingClientRect()
      const own = el.getBoundingClientRect()

      return { stacked: Math.abs(digest.top - own.top) > 4, offset: own.left - el.parentElement!.getBoundingClientRect().left }
    })

    if (seen.stacked) expect(seen.offset, `wrapped at ${width}px so it belongs at the start of its line`).toBeLessThan(3)
  }
})

test.describe('the versions table on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } })

  // Three columns never fit, so each keeps its width and the table scrolls rather than squeezing the dates.
  test('scrolls sideways instead of cramming the columns', async ({ page }) => {
    await page.goto(PAGE)

    const room = await page.locator('table').evaluate((el) => ({
      table: el.getBoundingClientRect().width,
      client: el.parentElement!.clientWidth,
      scroll: el.parentElement!.scrollWidth
    }))

    expect(room.table).toBeGreaterThan(room.client)
    expect(room.scroll).toBeGreaterThan(room.client)
  })
})
