import { execFileSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import type { Unstable_RawConfig as Config } from 'wrangler'
import { ASK_DOMAIN, BOT, BOT_DB, DISCORD, GUILD, SITE } from './names'

const LOCAL_DATABASE_ID = '00000000-0000-4000-8000-000000000001'

// Without deploy credentials, as in local dev and the check job, the binding stays local.
function database_id() {
  if (!process.env.CLOUDFLARE_API_TOKEN) return LOCAL_DATABASE_ID

  let databases: { name: string; uuid: string }[]

  try {
    databases = JSON.parse(execFileSync('npx', ['wrangler', 'd1', 'list', '--json'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }))
  } catch {
    throw new Error('Could not list D1 databases, check CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID.')
  }

  const database = databases.find((each) => each.name === BOT_DB)
  if (!database) throw new Error(`D1 "${BOT_DB}" does not exist yet. Create it with "npm run db" in bot.`)

  return database.uuid
}

const config: Config = {
  name: BOT,
  main: 'src/index.ts',
  compatibility_date: '2026-09-01',
  compatibility_flags: ['nodejs_compat'],
  // The engine the model runs code on, carried inside since a Worker cannot compile wasm while it runs.
  rules: [{ type: 'CompiledWasm', globs: ['**/*.wasm'], fallthrough: false }],
  observability: { enabled: true },
  workers_dev: false,
  preview_urls: false,
  routes: [{ pattern: ASK_DOMAIN, custom_domain: true }],
  vars: { GUILD, DISCORD, SITE },
  ai: { binding: 'AI' },
  // The day's budget is shared with the server, so one caller taking it all would silence the other door.
  ratelimits: [{ name: 'ASK_IP', namespace_id: '2001', simple: { limit: 5, period: 60 } }],
  d1_databases: [{ binding: 'DB', database_name: BOT_DB, database_id: database_id() }],
  durable_objects: { bindings: [{ name: 'BOT', class_name: 'Bot' }] },
  migrations: [{ tag: 'v1', new_sqlite_classes: ['Bot'] }],
  // The alarm drives the tick, so this only has to be often enough to re-arm one that never fired.
  triggers: { crons: ['*/5 * * * *'] }
}

writeFileSync(new URL('./wrangler.json', import.meta.url), JSON.stringify(config, null, 2) + '\n')
