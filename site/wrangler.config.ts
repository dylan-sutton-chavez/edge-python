import { execFileSync } from 'node:child_process'
import { writeFileSync } from 'node:fs'
import type { Unstable_RawConfig as Config } from 'wrangler'
import { ENV, WORKER, SITE_DOMAIN, SITE_URL, CDN_URL, DB_NAME, BUCKET, EMAIL_FROM } from '../infra/src/constants'

const LOCAL_DATABASE_ID = '00000000-0000-4000-8000-000000000000'

// Without deploy credentials, as in local dev and pull requests, the binding stays local.
function database_id() {
  if (!process.env.CLOUDFLARE_API_TOKEN) return LOCAL_DATABASE_ID

  let databases: { name: string; uuid: string }[]

  try {
    databases = JSON.parse(execFileSync('npx', ['wrangler', 'd1', 'list', '--json'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }))
  } catch {
    throw new Error('Could not list D1 databases, check CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID.')
  }

  const database = databases.find((each) => each.name === DB_NAME)
  if (!database) throw new Error(`D1 "${DB_NAME}" does not exist yet. Create it with "npm run app" in infra.`)

  return database.uuid
}

const config: Config = {
  name: WORKER,
  main: '@astrojs/cloudflare/entrypoints/server',
  compatibility_date: '2026-09-01',
  compatibility_flags: ['nodejs_compat', 'global_fetch_strictly_public'],
  assets: { directory: './dist', binding: 'ASSETS' },
  observability: { enabled: true },
  workers_dev: false,
  preview_urls: false,
  routes: [{ pattern: SITE_DOMAIN, custom_domain: true }],
  vars: { SITE: SITE_URL, CDN: CDN_URL, EMAIL_FROM, DRAFT: ENV === 'dev' ? '1' : '' }, // Empty outside dev, which is what turns a draft page into a 404.
  d1_databases: [{ binding: 'DB', database_name: DB_NAME, database_id: database_id(), migrations_dir: 'db/migrations' }],
  r2_buckets: [{ binding: 'CDN_BUCKET', bucket_name: BUCKET }],
  send_email: [{ name: 'EMAIL' }],
  ratelimits: [
    { name: 'OTP_IP', namespace_id: '1001', simple: { limit: 5, period: 60 } },
    { name: 'OTP_EMAIL', namespace_id: '1002', simple: { limit: 3, period: 60 } },
    { name: 'PUBLISH_NAME', namespace_id: '1003', simple: { limit: 2, period: 60 } },
    { name: 'PUBLISH_VERSION', namespace_id: '1004', simple: { limit: 30, period: 60 } }
  ]
}

writeFileSync(new URL('./wrangler.json', import.meta.url), JSON.stringify(config, null, 2) + '\n')
