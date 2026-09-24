import { globSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test as base, expect, type APIRequestContext, type APIResponse, type BrowserContext } from '@playwright/test'
import { random, sha256 } from '../src/lib/crypto'
import { BASE } from '../playwright.config'

export const MAILS = fileURLToPath(new URL('../.wrangler/tmp/email/', import.meta.url))

export const unique = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`

// Codes are rate limited per address, so every sign-in arrives from one of its own.
let arrival = 0
export const arriving = () => `10.0.0.${++arrival}`

/* Packs a tree the way `edge build` does, magic then each file as an ascii length, a newline and the bytes, so a publish test sends a real archive the route has to open. */
export function packed(files: Record<string, string>, entry = 'main.py') {
  const framed = (text: string) => {
    const body = Buffer.from(text)
    return Buffer.concat([Buffer.from(`${body.length}\n`), body])
  }

  const names = Object.keys(files)
  const parts = [Buffer.from('EDGEPKG\u0001', 'binary'), framed(entry), Buffer.from(`${names.length}\n`)]

  for (const path of names) parts.push(framed(path), framed(files[path]!))

  return Buffer.concat(parts)
}

// What wrangler will not retry itself, so a dropped connection surfaces as a 500 the assertion then blames on the route.
const MUTATES = ['post', 'patch', 'put', 'delete', 'fetch']

/* Sends again when the dev server drops the connection mid-flight. Nothing reached the worker, so nothing can be applied twice. A drop can outlast one immediate retry, so each wait is longer than the last before the status goes back for an assertion to judge. */
async function sent(send: () => Promise<APIResponse>, tries = 3) {
  let answer = await send()

  for (let at = 1; at < tries && answer.status() >= 500; at++) {
    await new Promise((wake) => setTimeout(wake, at * 250))
    answer = await send()
  }

  return answer
}

/* A context that retries what wrangler will not, so no test has to remember. Forty five mutating calls do not each need a wrapper, they need one that cannot be forgotten. */
export const retrying = (context: APIRequestContext): APIRequestContext =>
  new Proxy(context, {
    get(target, key) {
      const held = Reflect.get(target, key, target)
      if (typeof held !== 'function' || !MUTATES.includes(String(key))) return held

      return (...args: unknown[]) => sent(() => held.apply(target, args) as Promise<APIResponse>)
    }
  })

// Every spec takes its request context from here, which is the only place it can be wrapped once and for all.
export const test = base.extend({
  request: async ({ request }, use) => use(retrying(request))
})

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

/* A named account, ready to use. */
export async function signIn(request: APIRequestContext) {
  const email = `${unique()}@example.com`
  const handle = unique()

  await request.post('/api/auth/email/verify', { data: { email, code: await mailedCode(request, email, arriving()) } })
  await request.patch('/api/me', { data: { handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' } } })

  return { email, handle }
}

/* The same account through a browser context, which shares its cookies with the page, so a test lands signed in without walking the sign-up wizard its own tests already cover. Its own request context comes from the browser rather than the fixture, so it is wrapped here. */
export const signedIn = (context: BrowserContext) => signIn(retrying(context.request))

/* A usable token, minted the way the browser does so the secret exists only here. */
export async function mintToken(request: APIRequestContext, name = 'ci') {
  const secret = random(32)
  const salt = random(16)
  const hash = await sha256(salt + secret)

  const made = await request.post('/api/me/tokens', { data: { name, salt, hash } })
  expect(made.status()).toBe(201)

  return `edge_pat_${(await made.json()).id}.${secret}`
}

/* A package in the registry, published the way the CLI does, so a test that reads a page reads one that was really stored. */
export async function published(request: APIRequestContext, files: Record<string, string> = {}) {
  const account = await signIn(request)
  const token = await mintToken(request)
  const name = `p${unique()}`.toLowerCase().replace(/[^a-z0-9-]/g, '')

  const tree = { 'edge.json': JSON.stringify({ name, version: '0.1.0', ...DECLARED }), 'main.py': 'print(1)\n', ...files }
  const artifact = { name: 'app.edge', mimeType: 'application/octet-stream', buffer: packed(tree) }

  // A multipart post is a form submission, which the origin check covers, unlike the JSON posts above it.
  const headers = { authorization: `Bearer ${token}`, origin: BASE }
  const sent = await request.post('/api/publish', { headers, multipart: { artifact } })
  expect(sent.status(), await sent.text()).toBe(201)

  return { ...account, name }
}

const DECLARED = { description: 'Turn text into a slug.', repository: 'https://github.com/x/slugify' }
