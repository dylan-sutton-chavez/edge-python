import { test, expect } from '@playwright/test'
import { signedIn, unique } from './helpers'

const TOKEN = /^edge_pat_[A-Za-z0-9_-]+\.[A-Za-z0-9_-]{43}$/

test('mints a token in the browser, replaces it, and revokes it', async ({ page, context }) => {
  // Every body the page sends, so the secret can be looked for in all of them afterwards.
  const sent: string[] = []
  await page.route('**/api/**', async (route) => {
    sent.push(route.request().postData() ?? '')
    await route.continue()
  })

  await signedIn(context)
  await page.goto('/settings#tokens')

  await page.getByRole('button', { name: 'Create new' }).click()
  await page.locator('[name="token-name"]').fill('CI')
  await page.getByRole('button', { name: 'Generate' }).click()

  const shown = page.getByRole('dialog', { name: 'Please copy this token' })
  await expect(shown).toBeVisible()

  const token = (await shown.locator('[data-secret]').textContent()) ?? ''
  expect(token).toMatch(TOKEN)

  await shown.getByRole('button', { name: 'Close' }).click()

  // The row is there with no reload, which is what the template and the delegation buy.
  const rows = page.locator('[data-token]')
  await expect(rows).toHaveCount(1)
  await expect(rows.first()).toContainText('never used')

  // The secret was made in the page and only its hash was sent, so no request ever carried it.
  const secret = token.split('.')[1]!
  expect(sent.some((body) => body.includes(secret))).toBe(false)
  expect(sent.some((body) => body.includes('"hash"'))).toBe(true)

  await rows.first().getByRole('button', { name: 'Rotate' }).click()
  await expect(shown).toBeVisible()
  expect((await shown.locator('[data-secret]').textContent()) ?? '').not.toBe(token)
  await shown.getByRole('button', { name: 'Close' }).click()

  // Replacing leaves the old one working on a clock, and it can no longer be replaced again.
  await expect(rows).toHaveCount(2)
  await expect(rows.last()).toContainText('Replaced, stops working')
  await expect(rows.last().getByRole('button', { name: 'Rotate' })).toHaveCount(0)

  // Revoking asks first, and every way out of the dialog except the button means no.
  await rows.first().getByRole('button', { name: 'Revoke' }).click()

  const asks = page.getByRole('dialog', { name: 'Revoke CI?' })
  await expect(asks).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(rows).toHaveCount(2)

  await rows.first().getByRole('button', { name: 'Revoke' }).click()
  await asks.getByRole('button', { name: 'Revoke token' }).click()
  await expect(rows).toHaveCount(1)
})

test('asks before a handle moves and saves everything else quietly', async ({ page, context }) => {
  const { handle } = await signedIn(context)
  await page.goto('/settings')

  const asks = page.getByRole('dialog', { name: /Change your handle/ })
  const field = page.locator('#settings-handle')

  // A bio is nobody else's name, so it goes straight through. A save reloads, so wait for the fresh document.
  await page.locator('#settings-bio').fill('quietly edited')
  await Promise.all([page.waitForEvent('load'), page.getByRole('button', { name: 'Save changes' }).click()])

  await expect(asks).toBeHidden()
  await expect(page.locator('#settings-bio')).toHaveValue('quietly edited')

  const moved = unique()
  await field.fill(moved)
  await page.getByRole('button', { name: 'Save changes' }).click()

  // Backing out of the dialog leaves the handle where it was on the server.
  await expect(asks).toBeVisible()
  await page.keyboard.press('Escape')
  expect((await context.request.get(`/@${handle}`)).status()).toBe(200)

  await page.getByRole('button', { name: 'Save changes' }).click()
  await Promise.all([page.waitForEvent('load'), asks.getByRole('button', { name: 'Change handle' }).click()])

  await expect(field).toHaveValue(moved)
  expect((await context.request.get(`/@${moved}`)).status()).toBe(200)

  // A second move inside the week is refused, and the reason has to reach the field.
  await field.fill(unique())
  await page.getByRole('button', { name: 'Save changes' }).click()
  await asks.getByRole('button', { name: 'Change handle' }).click()

  await expect(page.locator('#settings-handle-error')).toContainText(/change your handle again in/)
})
