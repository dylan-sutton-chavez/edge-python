import { globSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, type APIRequestContext, type BrowserContext } from '@playwright/test'

export const MAILS = fileURLToPath(new URL('../.wrangler/tmp/email/', import.meta.url))

export const unique = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`

// Codes are rate limited per address, so every sign-in arrives from one of its own.
let arrival = 0
export const arriving = () => `10.0.0.${++arrival}`

/* The local email binding writes every message it sends, so the code is read rather than guessed. */
export async function mailedCode(request: APIRequestContext, email: string, from?: string) {
  const before = globSync(join(MAILS, '**/*.txt'))

  const started = await request.post('/api/auth/email/start', {
    data: { email },
    headers: from ? { 'cf-connecting-ip': from } : undefined
  })
  expect(await started.json()).toEqual({ ok: true })

  await expect.poll(() => globSync(join(MAILS, '**/*.txt')).length).toBe(before.length + 1)
  const mail = globSync(join(MAILS, '**/*.txt')).find((each) => !before.includes(each))!

  return readFileSync(mail, 'utf8').match(/\d{6}/)![0]
}

/* A named account, ready to use. The context shares its cookies with the page, so the browser lands signed in without walking the sign-up wizard, which its own tests already cover. */
export async function signedIn(context: BrowserContext) {
  const email = `${unique()}@example.com`
  const handle = unique()

  const code = await mailedCode(context.request, email, arriving())
  await context.request.post('/api/auth/email/verify', { data: { email, code } })
  await context.request.patch('/api/me', { data: { handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' } } })

  return { email, handle }
}
