import { expect, type Page } from '@playwright/test'
import { test } from './helpers'

const PAGE = '/docs/getting-started/introduction'

const editor = (page: Page) => page.locator('[data-input]')
const status = (page: Page) => page.locator('[data-status]')

// fill() writes .value straight through and the keydown handler never runs, so every edit here goes through real keys.
async function clear(page: Page) {
  const input = editor(page)
  await input.focus()
  await input.press('ControlOrMeta+a')
  await input.press('Backspace')
}

test.beforeEach(async ({ page }) => {
  await page.goto(PAGE)
  await expect(editor(page)).toBeVisible()
})

test('auto-pairs an opener and skips the closer', async ({ page }) => {
  const input = editor(page)
  await clear(page)

  await input.press('(')
  await expect(input).toHaveValue('()')

  await input.press('"')
  await expect(input).toHaveValue('("")')

  await input.press('"')
  await input.press(')')
  await expect(input).toHaveValue('("")')
})

test('inherits indentation and opens a level after a colon', async ({ page }) => {
  const input = editor(page)
  await clear(page)

  await input.pressSequentially('if True:')
  await input.press('Enter')
  await expect(input).toHaveValue('if True:\n  ')
})

test('indents and outdents a selection with Tab', async ({ page }) => {
  const input = editor(page)
  await clear(page)

  await input.pressSequentially('a')
  await input.press('Enter')
  await input.pressSequentially('b')
  await input.press('ControlOrMeta+a')

  await input.press('Tab')
  await expect(input).toHaveValue('  a\n  b')

  await input.press('Shift+Tab')
  await expect(input).toHaveValue('a\nb')
})

test('highlights the source with Shiki', async ({ page }) => {
  const tokens = page.locator('[data-view] span')

  await expect(tokens.first()).toBeAttached()
  expect(await tokens.count()).toBeGreaterThan(1)
})

test('scrolls the highlight with the source', async ({ page }) => {
  const input = editor(page)
  await clear(page)

  for (let i = 0; i < 8; i++) {
    await input.pressSequentially(`print(${i})`)
    await input.press('Enter')
  }

  const view = await input.evaluate((el: HTMLTextAreaElement) => {
    const overlay = el.previousElementSibling!
    return { scroll: el.scrollTop, overlayScroll: overlay.scrollTop }
  })

  expect(view.scroll).toBeGreaterThan(0)
  expect(view.overlayScroll).toBe(view.scroll)
})

test('runs the documented example and reports the elapsed time', async ({ page }) => {
  const expected = await page.locator('[data-playground]').getAttribute('data-expected')

  await page.getByRole('button', { name: 'Run' }).click()

  // The output is server rendered, so only the elapsed time tells us the run actually finished.
  await expect(status(page)).toHaveText(/^Output, \d+(\.\d+)?(ms|s)$/, { timeout: 30000 })
  await expect(page.locator('[data-output]')).toHaveText(expected!.trim())
})

test('runs from the keyboard without touching the source', async ({ page }) => {
  const input = editor(page)
  await input.focus()
  await input.press('ControlOrMeta+Enter')

  const source = await input.inputValue()
  await expect(status(page)).toHaveText(/^Output, /, { timeout: 30000 })
  await expect(input).toHaveValue(source)
})

test('reports a mismatch against the documented output', async ({ page }) => {
  const input = editor(page)
  await clear(page)
  await input.pressSequentially('print("something else"')

  await page.getByRole('button', { name: 'Run' }).click()
  await expect(status(page)).toHaveText('Output, differs', { timeout: 30000 })
})
