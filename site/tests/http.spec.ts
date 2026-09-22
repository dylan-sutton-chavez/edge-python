import { globSync, readdirSync, readFileSync } from 'node:fs'
import { join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test, expect, type APIRequestContext } from '@playwright/test'
import { MAILS, arriving, mailedCode, mintToken, unique } from './helpers'

const DOCS = fileURLToPath(new URL('../../docs/', import.meta.url))

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

async function signIn(request: APIRequestContext) {
  const email = `${unique()}@example.com`
  const handle = unique()

  await request.post('/api/auth/email/verify', { data: { email, code: await mailedCode(request, email, arriving()) } })
  await request.patch('/api/me', { data: { handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' } } })

  return { email, handle }
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
    await signIn(request)

    expect((await request.delete(`/api/me/tokens/${id}`)).status()).toBe(404)
    expect((await request.post('/api/me/tokens', { data: { name: 'CI', salt: SALT, hash: HASH, replaces: id } })).status()).toBe(404)
  })
})

test.describe('deleting an account', () => {
  test('turns a signed out visitor away', async ({ request }) => {
    const response = await request.delete('/api/me', { data: { code: '000000' } })

    expect(response.status()).toBe(401)
    expect(await response.json()).toEqual({ error: 'Not signed in.' })
  })

  // A live session is not enough, the code proves the mailbox still answers.
  test('refuses a session without a fresh code', async ({ request }) => {
    const { handle } = await signIn(request)

    for (const data of [{}, { code: '000000' }]) {
      const response = await request.delete('/api/me', { data })
      expect(response.status()).toBe(403)
      expect(await response.json()).toEqual({ error: 'That code is wrong or expired.' })
    }

    expect((await request.get(`/@${handle}`)).status()).toBe(200)
  })

  test('takes the account with the right code and signs the visitor out', async ({ request }) => {
    const { email, handle } = await signIn(request)

    const code = await mailedCode(request, email, arriving())
    expect(await (await request.delete('/api/me', { data: { code } })).json()).toEqual({ ok: true })

    expect((await request.get('/@' + handle)).status()).toBe(404)
    expect((await request.get('/settings', { maxRedirects: 0 })).headers().location).toBe('/')
  })
})

test.describe('changing a handle', () => {
  const profile = (handle: string) => ({ handle, name: 'Corpus', avatar: { icon: 1, palette: 'sky' } })

  // Naming yourself at sign-up is free, so the wait only starts once a handle actually moves.
  test('allows one move and then holds the handle for seven days', async ({ request }) => {
    await signIn(request)

    const held = unique()
    const moved = await request.patch('/api/me', { data: profile(held) })
    expect(moved.status()).toBe(200)
    expect((await moved.json()).handle).toBe(held)

    const again = await request.patch('/api/me', { data: profile(unique()) })
    expect(again.status()).toBe(429)
    expect((await again.json()).error).toMatch(/change your handle again in \d+ days?\./)

    // The rest of the profile still saves while the handle is held.
    const rest = await request.patch('/api/me', { data: { ...profile(held), bio: 'still editable' } })
    expect(rest.status()).toBe(200)
    expect(await (await request.get(`/@${held}`)).text()).toContain('still editable')
  })
})

test.describe('changing an address', () => {
  test('turns a signed out visitor away', async ({ request }) => {
    for (const response of [
      await request.post('/api/me/email/start', { data: { email: 'new@example.com' } }),
      await request.patch('/api/me/email', { data: { email: 'new@example.com', code: '000000' } })
    ]) {
      expect(response.status()).toBe(401)
      expect(await response.json()).toEqual({ error: 'Not signed in.' })
    }
  })

  test('refuses an address that is not one, its own, or one already held', async ({ request }) => {
    const { email } = await signIn(request)

    const bad = await request.post('/api/me/email/start', { data: { email: 'nope' } })
    expect(bad.status()).toBe(400)

    const same = await request.post('/api/me/email/start', { data: { email } })
    expect(same.status()).toBe(409)
    expect(await same.json()).toEqual({ error: 'That is already your address.' })

    const held = await request.post('/api/me/email/start', { data: { email: 'c.sutton.dylan@gmail.com' } })
    expect(held.status()).toBe(409)
    expect(await held.json()).toEqual({ error: 'Another account already uses that address.' })
  })

  test('needs the code mailed to the new address', async ({ request }) => {
    await signIn(request)

    const wanted = `${unique()}@example.com`
    expect(await (await request.post('/api/me/email/start', { data: { email: wanted } })).json()).toEqual({ ok: true })

    const wrong = await request.patch('/api/me/email', { data: { email: wanted, code: '000000' } })
    expect(wrong.status()).toBe(403)
    expect(await wrong.json()).toEqual({ error: 'That code is wrong or expired.' })
  })

  // The old address stops reaching the account, which is the whole point of keeping it in one place.
  test('moves where the codes go and leaves the old address out', async ({ request }) => {
    const { email: was, handle } = await signIn(request)
    const wanted = `${unique()}@example.com`

    const before = globSync(join(MAILS, '**/*.txt'))
    expect(await (await request.post('/api/me/email/start', { data: { email: wanted } })).json()).toEqual({ ok: true })

    await expect.poll(() => globSync(join(MAILS, '**/*.txt')).length).toBe(before.length + 1)
    const mail = globSync(join(MAILS, '**/*.txt')).find((each: string) => !before.includes(each))!
    const code = readFileSync(mail, 'utf8').match(/\d{6}/)![0]

    expect(await (await request.patch('/api/me/email', { data: { email: wanted, code } })).json()).toEqual({ email: wanted })
    expect(await (await request.get('/settings')).text()).toContain(wanted)

    await request.post('/api/auth/signout')

    // A code to the address it used to hold opens a fresh account, so it has no handle of its own.
    const again = await mailedCode(request, was, arriving())
    const back = await request.post('/api/auth/email/verify', { data: { email: was, code: again } })

    expect(await back.json()).toEqual({ ok: true, handle: null })
    expect((await request.get(`/@${handle}`)).status()).toBe(200)
  })
})

// A six-digit code is a million guesses, so the attempt counter is what makes it safe to mail one.
test('five wrong codes close an address for good', async ({ request }) => {
  const email = `${unique()}@example.com`
  const code = await mailedCode(request, email, arriving())

  for (let at = 1; at <= 5; at++) {
    const response = await request.post('/api/auth/email/verify', { data: { email, code: '000000' } })
    expect(await response.json(), `attempt ${at}`).toEqual({ ok: false, handle: null })
  }

  // Even the right code is refused now, so guessing cannot outlast the counter.
  expect(await (await request.post('/api/auth/email/verify', { data: { email, code } })).json())
    .toEqual({ ok: false, handle: null })
})

test.describe('publishing', () => {
  const artifact = { name: 'app.edge', mimeType: 'application/octet-stream', buffer: Buffer.from('EDGEPKG\u0001opaque to the registry') }
  const naming = () => `p${unique()}`.toLowerCase().replace(/[^a-z0-9-]/g, '')

  const send = (request: APIRequestContext, token: string, name: string, version: string) =>
    request.post('/api/publish', {
      headers: { authorization: `Bearer ${token}` },
      multipart: { manifest: JSON.stringify({ name, version }), artifact }
    })

  test('refuses anything without a live token', async ({ request }) => {
    for (const token of ['', 'edge_pat_nope.nope', 'not-a-token']) {
      const response = await send(request, token, naming(), '0.1.0')
      expect(response.status(), token).toBe(401)
    }
  })

  // One flow, because claiming a name and adding a version to it are the same request.
  test('claims a name, keeps the version, and refuses a repeat or a stranger', async ({ request }) => {
    await signIn(request)
    const token = await mintToken(request)
    const name = naming()

    const first = await send(request, token, name, '0.1.0')
    expect(first.status()).toBe(201)

    const { digest, url } = await first.json()
    expect(digest).toMatch(/^[0-9a-f]{64}$/)
    expect(url).toContain(`/pkg/${name}/0.1.0/app.edge`)

    // The same version never gets overwritten, a newer one is welcome.
    expect((await send(request, token, name, '0.1.0')).status()).toBe(409)
    expect((await send(request, token, name, '0.2.0')).status()).toBe(201)

    // What `edge add` reads, carrying the digest it will pin.
    const looked = await request.get(`/api/packages/${name}`)
    expect(looked.status()).toBe(200)
    expect(await looked.json()).toMatchObject({ name, version: '0.2.0' })

    // A name someone holds is theirs, and a shape the registry cannot serve is refused.
    await request.post('/api/auth/signout')
    await signIn(request)
    const other = await mintToken(request)

    expect((await send(request, other, name, '0.3.0')).status()).toBe(409)
    expect((await send(request, other, 'Upper', '0.1.0')).status()).toBe(400)
    expect((await send(request, other, naming(), '1.0')).status()).toBe(400)
  })

  test('says nothing is there for a package that was never published', async ({ request }) => {
    expect((await request.get(`/api/packages/${naming()}`)).status()).toBe(404)
  })
})
