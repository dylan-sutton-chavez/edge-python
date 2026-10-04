import { test, beforeEach } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { DatabaseSync } from 'node:sqlite'
import { stale } from './alarm'
import { cited } from './answer'
import { left, spend } from './budget'
import { HISTORY_MS, MAX_QUESTION, PER_DAY, SESSION_MS, TICK_MS, TURNS } from './config'
import { spoken } from './discord'
import { run } from './run'
import { search } from './search'
import { digest, link, remember, rooted, sweep, threaded, turns, type Turn } from './session'

const SCHEMA = readFileSync(new URL('../db/schema.sql', import.meta.url), 'utf8')

// D1 is SQLite, so the schema and the statements are the real ones and only the shape of a call is bridged.
function database(): D1Database {
  const db = new DatabaseSync(':memory:')
  db.exec(SCHEMA)

  const held = (sql: string, args: unknown[]): unknown => ({
    bind: (...bound: unknown[]) => held(sql, bound),
    run: async () => db.prepare(sql).run(...(args as never[])),
    first: async (column?: string) => {
      const row = db.prepare(sql).get(...(args as never[])) as Record<string, unknown> | undefined
      return row ? (column ? row[column] : row) : null
    }
  })

  return {
    prepare: (sql: string) => held(sql, []),
    batch: (all: { run: () => Promise<unknown> }[]) => Promise.all(all.map((each) => each.run()))
  } as unknown as D1Database
}

let env: { DB: D1Database }

beforeEach(() => {
  env = { DB: database() }
})

const said = (n: number): Turn[] => Array.from({ length: n }, (_, at) => ({ role: at % 2 ? 'assistant' : 'user', content: `turn ${at}` }) as Turn)

const aged = (id: string, last: number, born = Date.now()) =>
  env.DB.prepare('update session set last_at = ?, born_at = ? where id = ?').bind(last, born, id).run()

// The cap lives in the store, since one the public door could skip would not be a cap.
test('a conversation is capped however many turns are handed to it', async () => {
  const id = await digest('long')
  await remember(env, id, 'http', said(TURNS + 4))

  const held = await turns(env, id, 'http')
  assert.equal(held.length, TURNS)
  assert.equal(held.at(-1)!.content, `turn ${TURNS + 3}`)

  await remember(env, id, 'http', [{ role: 'user', content: 'x'.repeat(MAX_QUESTION * 3) }])
  assert.equal((await turns(env, id, 'http'))[0]!.content.length, MAX_QUESTION)
})

// Discord ids are public, so the digest of one is guessable and only the kind keeps a stranger out of the thread.
test('an id derived from Discord resolves nowhere else', async () => {
  const id = await rooted('100')
  await remember(env, id, 'discord', said(2))

  assert.equal((await turns(env, id, 'discord')).length, 2)
  assert.equal((await turns(env, await digest('discord:100'), 'http')).length, 0)
})

test('two threads of one person stay apart', async () => {
  const first = await rooted('100')
  const second = await rooted('102')

  await remember(env, first, 'discord', [{ role: 'user', content: 'declare a package' }])
  await link(env, '101', first)
  await remember(env, second, 'discord', [{ role: 'user', content: 'run the tests' }])
  await link(env, '103', second)

  assert.equal(await threaded(env, '101'), first)
  assert.equal(await threaded(env, '103'), second)
  assert.equal(await threaded(env, '999'), null)

  assert.equal((await turns(env, first, 'discord'))[0]!.content, 'declare a package')
  assert.equal((await turns(env, second, 'discord'))[0]!.content, 'run the tests')
})

// A caller holding an id from yesterday is not an error, so an ended conversation reads as a new one before the sweep takes it.
test('a conversation that went quiet starts over and is swept', async () => {
  const quiet = await digest('quiet')
  const old = await digest('old')

  await remember(env, quiet, 'http', said(2))
  await remember(env, old, 'http', said(2))
  await link(env, 'answer', quiet)

  await aged(quiet, Date.now() - SESSION_MS - 1)
  await aged(old, Date.now(), Date.now() - HISTORY_MS - 1)

  assert.equal((await turns(env, quiet, 'http')).length, 0)
  assert.equal((await turns(env, old, 'http')).length, 0)

  await sweep(env)

  assert.equal((await env.DB.prepare('select count(*) as n from session').first<number>('n')), 0)
  assert.equal(await threaded(env, 'answer'), null)
})

test('the day is bounded and the switch outlives it', async () => {
  assert.deepEqual(await left(env), { room: PER_DAY, quiet: false })

  await spend(env, 3)
  assert.equal((await left(env)).room, PER_DAY - 3)

  await spend(env, PER_DAY)
  assert.ok((await left(env)).room <= 0)

  // Silenced on a day long gone, so today opens silenced and with all of its budget.
  await env.DB.prepare("update budget set day = '2000-01-01', quiet = 1").run()
  assert.deepEqual(await left(env), { room: PER_DAY, quiet: true })
})

// Asking only whether an alarm existed left a stranded one standing, which is why the distance is what is checked.
test('an alarm that strayed from the next tick is armed again', () => {
  const now = Date.now()

  assert.equal(stale(now + TICK_MS, now), false, 'one tick ahead is healthy')
  assert.equal(stale(now - 1_000, now), false, 'just due is healthy')

  assert.equal(stale(null, now), true, 'none at all')
  assert.equal(stale(now - 10 * 60_000, now), true, 'behind and never delivered')
  assert.equal(stale(now + 10 * 60_000, now), true, 'pushed out by retries')
})

// Every term is searched, each page is read once as the part it points at, and one that fails to read keeps its snippet.
test('a search reads what it finds under /api', async (t) => {
  const site = 'https://edgepython.com'
  const groups = { title: 'Groups', where: 'Reference · Actors', href: '/docs/reference/actors#groups', snippet: '' }
  const served: Record<string, unknown> = {
    '/api/search?q=actors': { docs: [groups, { title: 'Gone', where: 'Docs', href: '/docs/gone', snippet: 'a \u0001group\u0002 of actors' }] },
    '/api/search?q=json': { docs: [groups], packages: [{ title: 'json', where: 'Package', href: '/package/json', snippet: '' }] },
    '/api/docs/reference/actors': { body: '# Actors\nIntro.\n## Groups\nA group runs one program.\n```python\n## not a heading\n```\n## Limits\nMemory.' },
    '/api/package/json': { name: 'json', version: '0.1.0' }
  }

  t.mock.method(globalThis, 'fetch', async (input: RequestInfo | URL) => {
    const path = String(input).slice(site.length)
    return path in served ? Response.json(served[path]) : new Response(null, { status: 404 })
  })

  assert.deepEqual(
    (await search(site, ['actors', 'json'])).map((each) => each.text),
    ['## Groups\nA group runs one program.\n```python\n## not a heading\n```', 'a group of actors', '{"name":"json","version":"0.1.0"}']
  )
})

// A page links once, after its first run even across lines, from the passage holding the words or else the term, and code and the model's numbers decide nothing.
test('each page is linked once, where its first run ends', () => {
  const page = (href: string, title: string, text = '') => ({ title, where: 'Reference · Functions', href, snippet: '', text })
  const memo = page('/docs/language/functions#memoization', 'Memoization', 'Pure functions are memoized after two calls with the same arguments, and purity is detected statically.')

  assert.deepEqual(
    cited(
      'https://edgepython.com',
      'Pure functions are memoized after two identical calls [2].\nPurity is detected statically [1]. Add a package with `edge add`. [see edge add]\n```python\nprint(xs[1])\n```\nIt is fast [reference] [3]. Purity is detected statically.',
      [memo],
      new Map([['edge add', page('/docs/reference/cli#edge-add', '`edge add`')]])
    ),
    {
      text: 'Pure functions are memoized after two identical calls.\nPurity is detected statically [1]. Add a package with `edge add` [2].\n```python\nprint(xs[1])\n```\nIt is fast. Purity is detected statically.',
      sources: ['https://edgepython.com/docs/language/functions#memoization', 'https://edgepython.com/docs/reference/cli#edge-add'],
      names: ['Memoization', 'edge add']
    }
  )
})

// What the model runs gets nothing granted and stops at its budget, so a loop it writes cannot stall a reply.
test('a run prints what it computes and stops where the engine says', () => {
  const engine = new WebAssembly.Module(readFileSync(new URL('../compiler.wasm', import.meta.url)))

  assert.deepEqual(run(engine, 'print(4 * 1024**3 // (31 * 1024))'), { output: '135300\n' })
  assert.match(run(engine, 'while True:\n    pass').error!, /budget exceeded/)
  assert.match(run(engine, 'import time').error!, /not provided/)
})

// Discord breaks a line after a block by itself, so a blank line written there shows as two.
test('an answer reads as one chat message', () => {
  const page = 'https://edgepython.com/docs/reference/actors'

  assert.equal(
    spoken({ text: 'Declare it [1].\n```python run\nprint(xs[1])\n```\n\n```text\nok\n```\n\nThen read `ys[1]`.\n\n\n---\nDone.', sources: [page], names: ['Actors'] }),
    `Declare it ([see Actors](<${page}>)).\n\n\`\`\`python\nprint(xs[1])\n\`\`\`\n\`\`\`text\nok\n\`\`\`\nThen read \`ys[1]\`.\n\nDone.`
  )

  assert.equal(
    spoken({ text: 'See [https://edgepython.com/docs](https://edgepython.com/docs) first.\n```python\nx = 1', sources: [], names: [] }),
    'See <https://edgepython.com/docs> first.\n\n```python\nx = 1\n```'
  )

  assert.equal(spoken({ text: 'This sentence is long enough to count. '.repeat(12).trim(), sources: [], names: [] }).split('\n\n').length, 2)
})
