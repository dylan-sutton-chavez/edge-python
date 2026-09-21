import { globSync, readdirSync, readFileSync } from 'node:fs'
import { join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test, expect, type APIRequestContext } from '@playwright/test'

const DOCS = fileURLToPath(new URL('../../docs/', import.meta.url))
const MAILS = fileURLToPath(new URL('../.wrangler/tmp/email/', import.meta.url))

const unique = () => `${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`

test.describe('pages', () => {
  test('answers the routes a visitor can reach', async ({ request }) => {
    expect((await request.get('/')).status()).toBe(200)
    expect((await request.get('/nobody-here')).status()).toBe(404)
    expect((await request.get('/docs/nope/nope')).status()).toBe(404)
  })

  test('sends /docs to the first page for good', async ({ request }) => {
    const response = await request.get('/docs', { maxRedirects: 0 })

    expect(response.status()).toBe(301)
    expect(response.headers().location).toBe('/docs/getting-started/introduction')
  })

  test('turns a signed out visitor away from the settings', async ({ request }) => {
    const response = await request.get('/settings', { maxRedirects: 0 })

    expect(response.status()).toBe(302)
    expect(response.headers().location).toBe('/')
  })

  test('says it is not found on the page itself', async ({ request }) => {
    expect(await (await request.get('/nobody-here')).text()).toContain('Not found')
  })
})

// Every page under docs/ renders once, with one heading and one playground per runnable fence.
test('renders every docs page with its playgrounds', async ({ request }) => {
  const walk = (dir: string): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
      entry.isDirectory() ? walk(join(dir, entry.name)) : entry.name.endsWith('.mdx') ? [join(dir, entry.name)] : []
    )

  const files = walk(DOCS)
  expect(files.length).toBeGreaterThan(0)

  for (const file of files) {
    const slug = relative(DOCS, file).split(sep).join('/').replace(/\.mdx$/, '').replace(/(^|\/)\d+-/g, '$1')
    const fences = (readFileSync(file, 'utf8').match(/^```edge-python\s*$/gm) ?? []).length

    const response = await request.get(`/docs/${slug}`)
    const html = await response.text()

    expect(response.status(), `/docs/${slug}`).toBe(200)
    expect((html.match(/<h1[\s>]/g) ?? []).length, `/docs/${slug} headings`).toBe(1)
    expect((html.match(/<div data-playground/g) ?? []).length, `/docs/${slug} playgrounds`).toBe(fences)
  }
})

test.describe('the api while signed out', () => {
  test('reports its health', async ({ request }) => {
    const body = await (await request.get('/api/health')).json()

    expect(body.ok).toBe(true)
    expect(body.users).toBeGreaterThan(0)
  })

  test('knows which handles are free', async ({ request }) => {
    expect(await (await request.get('/api/handles/dylan')).json()).toEqual({ available: false })
    expect(await (await request.get('/api/handles/docs')).json()).toEqual({ available: false })
    expect(await (await request.get(`/api/handles/${unique()}`)).json()).toEqual({ available: true })
  })

  test('refuses anything that needs an account', async ({ request }) => {
    for (const response of [await request.patch('/api/me'), await request.delete('/api/me/accounts/github')]) {
      expect(response.status()).toBe(401)
      expect(await response.json()).toEqual({ error: 'Not signed in.' })
    }
  })
})

test.describe('oauth without secrets', () => {
  test('says a provider is unconfigured rather than failing blind', async ({ request }) => {
    for (const provider of ['github', 'google']) {
      const response = await request.get(`/api/auth/${provider}`, { maxRedirects: 0 })
      expect(response.status()).toBe(503)
      expect(await response.text()).toContain(`Sign-in with ${provider} is not configured.`)
    }
  })

  test('rejects an unknown provider and a forged callback', async ({ request }) => {
    const unknown = await request.get('/api/auth/nope', { maxRedirects: 0 })
    expect(unknown.status()).toBe(404)
    expect(await unknown.text()).toContain('Unknown provider')

    const forged = await request.get('/api/auth/callback/github', { maxRedirects: 0 })
    expect(forged.status()).toBe(400)
    expect(await forged.text()).toContain('Invalid state')
  })
})

// The local email binding writes every message it sends, so the code is read rather than guessed.
async function mailedCode(request: APIRequestContext, email: string, from?: string) {
  const before = globSync(join(MAILS, '**/*.txt'))

  // Codes are rate limited per address, so a test that signs in twice arrives from a second one.
  const started = await request.post('/api/auth/email/start', {
    data: { email },
    headers: from ? { 'cf-connecting-ip': from } : undefined
  })
  expect(await started.json()).toEqual({ ok: true })

  await expect.poll(() => globSync(join(MAILS, '**/*.txt')).length).toBe(before.length + 1)
  const mail = globSync(join(MAILS, '**/*.txt')).find((each) => !before.includes(each))!

  return readFileSync(mail, 'utf8').match(/\d{6}/)![0]
}

test.describe('signing in by email', () => {
  test('turns down an address that is not one', async ({ request }) => {
    const response = await request.post('/api/auth/email/start', { data: { email: 'nope' } })

    expect(response.status()).toBe(400)
    expect(await response.json()).toEqual({ error: 'Enter a valid email.' })
  })

  test('takes an account from a code to a profile and back out', async ({ request }) => {
    const email = `${unique()}@example.com`
    const handle = unique()
    const code = await mailedCode(request, email)

    expect(await (await request.post('/api/auth/email/verify', { data: { email, code: 'not-it' } })).json())
      .toEqual({ ok: false, handle: null })
    expect(await (await request.post('/api/auth/email/verify', { data: { email, code } })).json())
      .toEqual({ ok: true, handle: null })

    const welcome = await request.get('/settings', { maxRedirects: 0 })
    expect(welcome.headers().location).toBe('/?welcome')

    const taken = await request.patch('/api/me', { data: { handle: 'dylan', name: 'Corpus', avatar: { icon: 1, palette: 'sky' } } })
    expect(taken.status()).toBe(409)
    expect(await taken.json()).toEqual({ error: '@dylan is already taken.' })

    const saved = await request.patch('/api/me', { data: { handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' }, bio: 'hello' } })
    expect(saved.status()).toBe(200)
    expect((await saved.json()).handle).toBe(handle)

    const profile = await request.get(`/@${handle}`)
    expect(profile.status()).toBe(200)
    expect(await profile.text()).toContain('hello')

    expect((await request.get('/settings', { maxRedirects: 0 })).status()).toBe(200)

    expect(await (await request.post('/api/auth/signout')).json()).toEqual({ ok: true })
    expect((await request.get('/settings', { maxRedirects: 0 })).headers().location).toBe('/')
  })
})

// The browser makes the secret, so a request only ever carries shapes the server can check.
const b64url = (length: number) =>
  Array.from({ length }, (_, at) => 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_'[at % 64]).join('')

const SALT = b64url(22)
const HASH = b64url(43)

async function signIn(request: APIRequestContext, from?: string) {
  const email = `${unique()}@example.com`
  const handle = unique()

  await request.post('/api/auth/email/verify', { data: { email, code: await mailedCode(request, email, from) } })
  await request.patch('/api/me', { data: { handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' } } })

  return handle
}

test.describe('publish tokens', () => {
  test('turns a signed out visitor away', async ({ request }) => {
    for (const response of [
      await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH } }),
      await request.delete('/api/me/tokens/whatever')
    ]) {
      expect(response.status()).toBe(401)
      expect(await response.json()).toEqual({ error: 'Not signed in.' })
    }
  })

  test('refuses a name or a digest it cannot have produced', async ({ request }) => {
    await signIn(request)

    const nameless = await request.post('/api/me/tokens', { data: { name: '   ', salt: SALT, hash: HASH } })
    expect(nameless.status()).toBe(400)
    expect(await nameless.json()).toEqual({ error: 'Name it in 40 characters or fewer.' })

    for (const bad of [{ salt: 'short', hash: HASH }, { salt: SALT, hash: 'short' }, { salt: SALT, hash: `${HASH.slice(1)}.` }]) {
      const response = await request.post('/api/me/tokens', { data: { name: 'CI', ...bad } })
      expect(response.status()).toBe(400)
      expect(await response.json()).toEqual({ error: 'That token was not generated here.' })
    }
  })

  test('creates one, replaces it on a clock, and revokes', async ({ request }) => {
    await signIn(request)

    const made = await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH } })
    expect(made.status()).toBe(201)
    const { id } = await made.json()
    expect(id).toBeTruthy()

    const listed = await (await request.get('/settings')).text()
    expect(listed).toContain('CI')
    expect(listed).toContain('never used')

    const replaced = await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH, replaces: id } })
    expect(replaced.status()).toBe(201)
    const second = (await replaced.json()).id
    expect(second).not.toBe(id)

    // The replaced token is still live, it just has a day left, which is what keeps a deploy from breaking.
    const both = await (await request.get('/settings')).text()
    expect(both).toContain('Replaced, stops working')

    expect(await (await request.delete(`/api/me/tokens/${id}`)).json()).toEqual({ ok: true })
    expect((await request.delete(`/api/me/tokens/${id}`)).status()).toBe(404)
    expect(await (await request.delete(`/api/me/tokens/${second}`)).json()).toEqual({ ok: true })
  })

  test('replacing a token nobody owns finds nothing', async ({ request }) => {
    await signIn(request)

    const response = await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH, replaces: 'nope42' } })
    expect(response.status()).toBe(404)
    expect(await response.json()).toEqual({ error: 'No such token.' })
  })

  test('another account cannot revoke or replace your token', async ({ request }) => {
    await signIn(request)
    const { id } = await (await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH } })).json()

    await request.post('/api/auth/signout')
    await signIn(request, '10.0.0.2')

    expect((await request.delete(`/api/me/tokens/${id}`)).status()).toBe(404)
    expect((await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH, replaces: id } })).status()).toBe(404)
  })
})
