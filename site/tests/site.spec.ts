import { test, expect, type Page } from '@playwright/test'

const routes = [
  { path: '/', group: 'Explore', label: 'Community' },
  { path: '/docs/getting-started/introduction', group: 'Docs', label: 'Getting started' }
]

const nav = (page: Page) => page.locator('header nav[aria-label="Main"]')
const trigger = (page: Page, group: string) => nav(page).locator('[data-nav-trigger]', { hasText: group })
const panel = (page: Page, group: string) => nav(page).locator('[data-nav]', { hasText: group }).locator('[data-nav-panel]')

for (const { path, group, label } of routes) {
  test(`marks ${label} as the current page`, async ({ page }) => {
    await page.goto(path)

    await expect(panel(page, group).locator('[aria-current="page"]')).toHaveAttribute('href', path)

    await expect(page.locator('header')).toBeVisible()
    await expect(page.locator('footer')).toBeVisible()
  })
}

test.describe('the header nav', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
  })

  test('slides the pages open when the cursor rests on a group', async ({ page }) => {
    await expect(panel(page, 'Docs')).toBeHidden()

    await trigger(page, 'Docs').hover()

    await expect(panel(page, 'Docs')).toBeVisible()
    await expect(trigger(page, 'Docs')).toHaveAttribute('aria-expanded', 'true')
  })

  test('stays open while the cursor crosses the gap into the panel', async ({ page }) => {
    await trigger(page, 'Docs').hover()
    await expect(panel(page, 'Docs')).toBeVisible()

    const box = (await panel(page, 'Docs').boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2, { steps: 12 })

    await page.waitForTimeout(400)
    await expect(panel(page, 'Docs')).toBeVisible()
  })

  test('swaps panels when the cursor slides across the nav', async ({ page }) => {
    await trigger(page, 'Docs').hover()
    await expect(panel(page, 'Docs')).toBeVisible()

    await trigger(page, 'Explore').hover()
    await expect(panel(page, 'Explore')).toBeVisible()
    await expect(panel(page, 'Docs')).toBeHidden()
  })

  test('closes once the cursor leaves', async ({ page }) => {
    await trigger(page, 'Docs').hover()
    await expect(panel(page, 'Docs')).toBeVisible()

    await page.locator('footer').hover()
    await expect(panel(page, 'Docs')).toBeHidden()
  })

  test('opens from the keyboard and closes on Escape', async ({ page }) => {
    await trigger(page, 'Explore').focus()
    await expect(panel(page, 'Explore')).toBeVisible()

    await page.keyboard.press('Escape')
    await expect(panel(page, 'Explore')).toBeHidden()
    await expect(trigger(page, 'Explore')).toHaveAttribute('aria-expanded', 'false')
  })

  test('navigates to the page it names', async ({ page }) => {
    await trigger(page, 'Docs').hover()
    await panel(page, 'Docs').getByRole('link', { name: 'Language' }).click()

    await expect(page).toHaveURL('/docs/language/syntax')
  })
})

test.describe('the header nav on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true })

  const bar = (page: Page) => page.getByRole('button', { name: 'Open navigation' })
  const sheet = (page: Page) => page.locator('#main-nav')

  test.beforeEach(async ({ page }) => {
    await page.goto('/')
  })

  test('collapses the groups into a bar', async ({ page }) => {
    await expect(trigger(page, 'Docs')).toBeHidden()
    await expect(bar(page)).toHaveText('Menu')
    await expect(sheet(page)).toBeHidden()
  })

  test('raises a sheet naming every group', async ({ page }) => {
    await bar(page).tap()
    await expect(sheet(page)).toBeVisible()

    for (const { group } of routes) {
      await expect(sheet(page).getByText(group, { exact: true })).toBeVisible()
    }
  })

  // The groups are one accordion, so only the section the visitor opens shows its pages.
  test('opens a group to reveal its pages', async ({ page }) => {
    await bar(page).tap()
    await expect(sheet(page).getByRole('link', { name: 'Getting started', exact: true })).toBeHidden()

    await sheet(page).getByText('Docs', { exact: true }).tap()

    await expect(sheet(page).getByRole('link', { name: 'Getting started', exact: true })).toBeVisible()
  })

  test('leaves the groups unclickable', async ({ page }) => {
    await bar(page).tap()

    for (const { group } of routes) {
      await expect(sheet(page).getByRole('link', { name: group, exact: true })).toHaveCount(0)
    }
  })

  test('closes when a page is picked and lands on it', async ({ page }) => {
    await bar(page).tap()
    await sheet(page).getByText('Docs', { exact: true }).tap()
    await sheet(page).getByRole('link', { name: 'Getting started', exact: true }).tap()

    await expect(sheet(page)).toBeHidden()
    await expect(page).toHaveURL('/docs/getting-started/introduction')
  })
})

test('toggles the theme and remembers the choice', async ({ page }) => {
  await page.goto('/')

  const html = page.locator('html')
  const before = await html.getAttribute('data-theme')
  const after = before === 'light' ? 'dark' : 'light'

  await page.locator('header > div > [data-theme-toggle]').click()
  await expect(html).toHaveAttribute('data-theme', after)

  await page.reload()
  await expect(html).toHaveAttribute('data-theme', after)
})

test('takes the theme from the device until a choice is stored', async ({ page }) => {
  const html = page.locator('html')

  // Nothing stored, so the inline script has only the device to read.
  await page.emulateMedia({ colorScheme: 'dark' })
  await page.goto('/')
  await expect(html).toHaveAttribute('data-theme', 'dark')

  // Still nothing stored, so a device that changes its mind is followed without a reload.
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(html).toHaveAttribute('data-theme', 'light')

  await page.locator('header > div > [data-theme-toggle]').click()
  await expect(html).toHaveAttribute('data-theme', 'dark')

  // A stored choice outranks the device from then on.
  await page.emulateMedia({ colorScheme: 'light' })
  await expect(html).toHaveAttribute('data-theme', 'dark')
})

// Tailwind v4 gates hover: behind @media (hover: hover); this fails if that override is ever dropped.
test('applies hover styles', async ({ page }) => {
  await page.goto('/')

  const link = trigger(page, 'Docs')
  const color = () => link.evaluate((el) => getComputedStyle(el).color)

  const idle = await color()
  await link.hover()

  await expect.poll(color).not.toBe(idle)
})

test.describe('dialogs', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
  })

  test('opens the sign-in dialog from the header button', async ({ page }) => {
    const dialog = page.getByRole('dialog', { name: 'Sign In' })
    await expect(dialog).toBeHidden()

    await page.getByRole('button', { name: 'Sign in', exact: true }).click()
    await expect(dialog).toBeVisible()
    await expect(dialog.getByLabel('Email')).toBeVisible()

    await dialog.getByRole('button', { name: 'Close' }).click()
    await expect(dialog).toBeHidden()
  })

  test('opens the agent briefing and closes it on Escape', async ({ page }) => {
    const dialog = page.getByRole('dialog', { name: 'Connect an agent' })

    await page.locator('[data-dialog-open="agent"]').click()
    await expect(dialog).toBeVisible()
    await expect(dialog.locator('[data-briefing]')).toContainText('edge.json')

    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
  })
})
