import { mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, extname, join, relative, sep } from 'node:path'
import { brotliCompressSync, constants } from 'node:zlib'
import Cloudflare from 'cloudflare'
import { account_id, client, zone_id } from '../client'
import { BUCKET, CDN_DOMAIN, ENV, TMP_BUCKET, TMP_CDN_DOMAIN, TMP_CDN_URL, TMP_EXPIRY_SECONDS, ZONE, keeps } from '../constants'

const TYPES: Record<string, string> = {
  '.wasm': 'application/wasm',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.py': 'text/x-python; charset=utf-8',
  '.sh': 'text/x-shellscript; charset=utf-8',
  '.gz': 'application/gzip',
  '.ts': 'text/plain; charset=utf-8'
}

// Paths carry no version, so every object revalidates against its ETag.
export const CACHE = 'public, max-age=0, must-revalidate'

// A versioned tree never changes, and a year is the longest a cache accepts.
export const FROZEN_CACHE = 'public, max-age=31536000, immutable'

const FROZEN = /^v\d+\.\d+\.\d+\//

export const cache_control = (prefix: string) => (FROZEN.test(prefix) ? FROZEN_CACHE : CACHE)

// A release freezes a copy of its own from 1.0, since 0.x promises nothing the storage buys.
export function frozen_prefix(release: string) {
  const version = release.replace(/^v/, '')
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`"${release}" is not a release, pass one shaped vX.Y.Z.`)

  return Number(version.split('.')[0]) >= 1 ? `v${version}/` : ''
}

// The compiler is the large payload, stored brotli encoded.
const encoded = (key: string) => key === 'compiler.wasm'

// Build inputs later CI jobs read from tmp, promote never ships them.
export const INTERNAL = '_build/'

export type CdnObject = { key: string; file: string; type: string; encoding: string | null }

async function ensure_bucket(bucket: string, domain: string, expiry_seconds?: number) {
  try {
    await client.r2.buckets.get(bucket, { account_id })
  } catch (error) {
    if (!(error instanceof Cloudflare.NotFoundError)) throw error

    console.log(`Creating R2 bucket "${bucket}"...`)
    await client.r2.buckets.create({ account_id, name: bucket })
  }

  const { domains } = await client.r2.buckets.domains.custom.list(bucket, { account_id })
  const found = domains.find((each) => each.domain === domain)

  if (!found) {
    console.log(`Attaching "${domain}" to R2 bucket "${bucket}"...`)
    await client.r2.buckets.domains.custom.create(bucket, { account_id, domain, zoneId: await zone_id(ZONE), enabled: true, minTLS: '1.2' })
  } else if (!found.enabled) {
    console.log(`Enabling "${domain}" on R2 bucket "${bucket}"...`)
    await client.r2.buckets.domains.custom.update(domain, { account_id, bucket_name: bucket, enabled: true })
  } else console.log(`R2 custom domain "${domain}" already enabled.`)

  // Web pages and the JS host fetch from other origins, reads only.
  await client.r2.buckets.cors.update(bucket, { account_id, rules: [{ allowed: { origins: ['*'], methods: ['GET', 'HEAD'] }, maxAgeSeconds: 86400 }] })

  if (expiry_seconds) {
    // Runs nobody promoted age out on their own.
    await client.r2.buckets.lifecycle.update(bucket, {
      account_id,
      rules: [{ id: 'expire-runs', enabled: true, conditions: { prefix: '' }, deleteObjectsTransition: { condition: { type: 'Age', maxAge: expiry_seconds } } }]
    })
  }

  await until_live(domain)
}

async function until_live(domain: string, timeout_ms = 10 * 60_000) {
  const deadline = Date.now() + timeout_ms

  for (let first = true; ; first = false) {
    try {
      const response = await fetch(`https://${domain}/`, { method: 'HEAD' })
      if (response.status < 500) return
    } catch {}

    if (Date.now() > deadline) throw new Error(`"${domain}" did not start serving within ${timeout_ms / 60_000} minutes.`)
    if (first) console.log(`Waiting for "${domain}" to start serving...`)
    await new Promise((resolve) => setTimeout(resolve, 10_000))
  }
}

export const ensure_site_cdn = () => ensure_bucket(BUCKET, CDN_DOMAIN)

// Respect Existing Headers, so the zone's four hour default never overrides CACHE.
export const ensure_browser_cache = async () => client.zones.settings.edit('browser_cache_ttl', { zone_id: await zone_id(ZONE), value: 0 })

// The edge holds what a route called public, so a shared address asking again never wakes the Worker.
export const ensure_read_cache = async () =>
  client.rulesets.phases.update('http_request_cache_settings', {
    zone_id: await zone_id(ZONE),
    rules: [
      {
        action: 'set_cache_settings',
        expression:
          '(http.request.uri.path in {"/api/search" "/api/docs" "/SKILL.md"} or starts_with(http.request.uri.path, "/api/package/") or starts_with(http.request.uri.path, "/api/program/") or starts_with(http.request.uri.path, "/api/docs/") or starts_with(http.request.uri.path, "/api/@") or starts_with(http.request.uri.path, "/api/resolve/"))',
        action_parameters: { cache: true, edge_ttl: { mode: 'respect_origin' } }
      }
    ]
  }).catch((error) => {
    if ((error as { status?: number }).status !== 403) throw error
    throw new Error(`CLOUDFLARE_API_TOKEN cannot write cache rules on "${ZONE}". Add the "Cache Rules" zone permission with edit access.`)
  })

export const ensure_tmp_cdn = () => ensure_bucket(TMP_BUCKET, TMP_CDN_DOMAIN, TMP_EXPIRY_SECONDS)

export function cdn_objects(tree: string): CdnObject[] {
  const walk = (dir: string): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((entry) => (entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)]))

  return walk(tree)
    .map((file) => {
      const key = relative(tree, file).split(sep).join('/')
      return { key, file, type: TYPES[extname(key)] ?? 'application/octet-stream', encoding: encoded(key) ? 'br' : null }
    })
    .sort((a, b) => a.key.localeCompare(b.key))
}

// The bytes an object stores, a runtime of tens of megabytes compresses at a quicker level.
function object_bytes(object: CdnObject) {
  const bytes = readFileSync(object.file)
  const quality = bytes.length > 8 << 20 ? 9 : 11
  return object.encoding === 'br' ? brotliCompressSync(bytes, { params: { [constants.BROTLI_PARAM_QUALITY]: quality } }) : bytes
}

// Keys keep their slashes in the request path, the way wrangler sends them.
const object_path = (bucket: string, key: string) => `/accounts/${account_id}/r2/buckets/${bucket}/objects/${key.split('/').map(encodeURIComponent).join('/')}`

// A few requests in flight, a CDN tree is dozens of small objects.
async function pool<T>(items: T[], task: (item: T) => Promise<void>, width = 8) {
  const queue = [...items]
  const worker = async () => {
    for (let item = queue.shift(); item !== undefined; item = queue.shift()) await task(item)
  }
  await Promise.all(Array.from({ length: Math.min(width, queue.length) }, worker))
}

export async function put_tree(bucket: string, prefix: string, tree: string) {
  const objects = cdn_objects(tree)
  const cache = cache_control(prefix)

  await pool(objects, async (object) => {
    console.log(`Uploading "${prefix}${object.key}" (${object.type}${object.encoding ? `, ${object.encoding}` : ''})...`)
    const headers = { 'Content-Type': object.type, 'Cache-Control': cache, ...(object.encoding ? { 'Content-Encoding': object.encoding } : {}) }
    await client.put(object_path(bucket, `${prefix}${object.key}`), { body: object_bytes(object), headers })
  })

  return objects.map((each) => each.key)
}

export async function list_keys(bucket: string, prefix?: string) {
  const keys: string[] = []
  for await (const object of client.r2.buckets.objects.list(bucket, { account_id, ...(prefix ? { prefix } : {}) })) {
    if (object.key) keys.push(object.key)
  }
  return keys
}

export async function delete_keys(bucket: string, keys: string[]) {
  await pool(keys, async (key) => {
    console.log(`Deleting "${key}" from "${bucket}"...`)
    await client.delete(object_path(bucket, key))
  })
}

// A promote replaces the whole tree, sparing a frozen release and the packages its environment keeps.
export function swept(keys: string[], shipped: string[], env: string) {
  const keep = new Set(shipped)
  return keys.filter((key) => !keep.has(key) && !FROZEN.test(key) && !(keeps(env) && key.startsWith('pkg/')))
}

export async function prune(bucket: string, shipped: string[]) {
  await delete_keys(bucket, swept(await list_keys(bucket), shipped, ENV))
}

// Downloads a run's public tree from tmp, decoded, minus the build inputs.
export async function pull(run: string) {
  const tree = mkdtempSync(join(tmpdir(), 'promote-'))
  const keys = (await list_keys(TMP_BUCKET, `${run}/`)).map((key) => key.slice(run.length + 1)).filter((key) => !key.startsWith(INTERNAL))
  if (!keys.length) throw new Error(`Nothing is staged under "${TMP_CDN_URL}/${run}/".`)

  await pool(keys, async (key) => {
    const response = await fetch(`${TMP_CDN_URL}/${run}/${key}`)
    if (!response.ok) throw new Error(`Fetching "${run}/${key}" from tmp answered ${response.status}.`)

    const file = join(tree, ...key.split('/'))
    mkdirSync(dirname(file), { recursive: true })
    writeFileSync(file, Buffer.from(await response.arrayBuffer()))
  })

  return tree
}
