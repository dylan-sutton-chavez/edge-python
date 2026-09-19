import { spawn } from 'node:child_process'
import { readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { isDeepStrictEqual } from 'node:util'

type Step = { method?: string; path: string; json?: unknown; status: number; location?: string; equals?: unknown; contains?: string[] }
type Flow = { name: string; steps?: Step[]; docs?: boolean }

const SITE = fileURLToPath(new URL('../', import.meta.url))
const DOCS = fileURLToPath(new URL('../../docs/', import.meta.url))
const WRANGLER = join(dirname(createRequire(import.meta.url).resolve('wrangler/package.json')), 'bin/wrangler.js')
const PORT = process.env.PORT ?? '8787'
const BASE = `http://127.0.0.1:${PORT}`

// Unique per run, so the sign-in flow never meets the seed or an earlier run.
const stamp = Date.now().toString(36)
const vars: Record<string, string> = { email: `corpus-${stamp}@example.com`, handle: `corpus-${stamp}` }

// The built worker on local bindings, its email binding prints every code it sends.
const server = spawn(process.execPath, [WRANGLER, 'dev', '--ip', '127.0.0.1', '--port', PORT], { cwd: SITE, stdio: ['ignore', 'pipe', 'pipe'] })
let log = ''
for (const stream of [server.stdout, server.stderr]) stream.on('data', (chunk) => (log += chunk))

const sleep = (ms: number) => new Promise((done) => setTimeout(done, ms))

async function ready() {
  for (let i = 0; i < 120; i++) {
    if (server.exitCode !== null) throw new Error(`wrangler dev exited\n${log}`)
    try {
      if ((await fetch(`${BASE}/api/health`)).ok) return
    } catch {}
    await sleep(500)
  }
  throw new Error(`wrangler dev never answered on ${BASE}\n${log}`)
}

async function code() {
  const sent = new RegExp(`To: ${vars.email.replaceAll('.', '\\.')}\\s+Subject: (\\d{6}) is your Edge Python code`)
  for (let i = 0; i < 50; i++) {
    const found = log.match(sent)
    if (found) return found[1]!
    await sleep(200)
  }
  throw new Error(`no code was mailed to ${vars.email}`)
}

async function fill(text: string) {
  let out = text
  for (const [name, value] of Object.entries(vars)) out = out.replaceAll(`{${name}}`, value)
  return out.includes('{code}') ? out.replaceAll('{code}', await code()) : out
}

async function fill_json(value: unknown): Promise<unknown> {
  if (typeof value === 'string') return fill(value)
  if (Array.isArray(value)) return Promise.all(value.map(fill_json))
  if (value && typeof value === 'object') return Object.fromEntries(await Promise.all(Object.entries(value).map(async ([key, each]) => [key, await fill_json(each)])))
  return value
}

// Cookies the site sets, sent back on every later step of the same flow.
const jar = new Map<string, string>()

function remember(response: Response) {
  for (const cookie of response.headers.getSetCookie()) {
    const [pair = '', ...attributes] = cookie.split(';')
    const name = pair.slice(0, pair.indexOf('=')).trim()
    const value = pair.slice(pair.indexOf('=') + 1).trim()
    const expired = attributes.some((each) => /^\s*max-age=0\s*$/i.test(each) || (/^\s*expires=/i.test(each) && Date.parse(each.split('=')[1] ?? '') < Date.now()))
    if (expired || !value) jar.delete(name)
    else jar.set(name, value)
  }
}

async function step(each: Step) {
  const method = each.method ?? 'GET'
  const path = await fill(each.path)
  const headers: Record<string, string> = { origin: BASE }
  if (jar.size) headers.cookie = [...jar].map(([name, value]) => `${name}=${value}`).join('; ')
  if (each.json !== undefined) headers['content-type'] = 'application/json'

  const body = each.json === undefined ? undefined : JSON.stringify(await fill_json(each.json))
  const response = await fetch(BASE + path, { method, headers, body, redirect: 'manual' })
  remember(response)
  const text = await response.text()

  const problems: string[] = []
  if (response.status !== each.status) problems.push(`status ${response.status}, want ${each.status}`)
  if (each.location !== undefined && response.headers.get('location') !== (await fill(each.location))) problems.push(`location ${response.headers.get('location')}, want ${await fill(each.location)}`)

  if (each.equals !== undefined) {
    let got: unknown = text
    try {
      got = JSON.parse(text)
    } catch {}
    const want = await fill_json(each.equals)
    if (!isDeepStrictEqual(got, want)) problems.push(`body ${JSON.stringify(got)}, want ${JSON.stringify(want)}`)
  }

  for (const needle of each.contains ?? []) {
    if (!text.includes(await fill(needle))) problems.push(`body lacks ${JSON.stringify(await fill(needle))}`)
  }

  return problems.map((problem) => `${method} ${path}: ${problem}`)
}

// Every page under docs/ renders once, with one heading and one playground per runnable fence.
async function sweep() {
  const walk = (dir: string): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((entry) => (entry.isDirectory() ? walk(join(dir, entry.name)) : entry.name.endsWith('.mdx') ? [join(dir, entry.name)] : []))

  const problems: string[] = []
  for (const file of walk(DOCS)) {
    const slug = relative(DOCS, file).split(sep).join('/').replace(/\.mdx$/, '').replace(/(^|\/)\d+-/g, '$1')
    const fences = (readFileSync(file, 'utf8').match(/^```edge-python\s*$/gm) ?? []).length
    const response = await fetch(`${BASE}/docs/${slug}`)
    const html = await response.text()
    const headings = (html.match(/<h1[\s>]/g) ?? []).length
    const playgrounds = (html.match(/<div data-playground/g) ?? []).length

    if (response.status !== 200) problems.push(`/docs/${slug}: status ${response.status}, want 200`)
    if (headings !== 1) problems.push(`/docs/${slug}: ${headings} h1 headings, want 1`)
    if (playgrounds !== fences) problems.push(`/docs/${slug}: ${playgrounds} playgrounds, want ${fences}`)
  }
  return problems
}

const flows: Flow[] = JSON.parse(readFileSync(new URL('./routes.json', import.meta.url), 'utf8'))
const failures: string[] = []

try {
  await ready()
  for (const flow of flows) {
    jar.clear()
    const problems: string[] = []
    if (flow.docs) problems.push(...(await sweep()))
    for (const each of flow.steps ?? []) problems.push(...(await step(each)))

    console.log(`${problems.length ? 'FAIL' : 'ok  '} ${flow.name}`)
    failures.push(...problems.map((problem) => `[${flow.name}] ${problem}`))
  }
} finally {
  server.kill('SIGTERM')
}

if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
