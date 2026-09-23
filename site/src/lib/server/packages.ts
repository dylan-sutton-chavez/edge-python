import { check } from '../docs/convention'
import { identify } from './license'

export type Package = { name: string; user_id: string | null; created_at: number }

// One page a bundle carried, keyed by the path the renderer orders it with.
export type Page = { path: string; body: string }

export type Release = {
  name: string
  version: string
  digest: string
  size: number
  description: string | null
  repository: string | null
  notice: string | null
  hosts: string
  docs: Page[]
}

const NAME = /^[a-z][a-z0-9-]*$/
const VERSION = /^\d{1,9}\.\d{1,9}\.\d{1,9}$/

export const MAX_NAME = 40
export const MAX_ARTIFACT = 32 << 20
export const MAX_DESCRIPTION = 200
export const MAX_REPOSITORY = 256
export const MAX_NOTICE = 64 << 10
export const MAX_PAGES = 64
export const MAX_PAGE = 128 << 10

// The hosts an artifact can name, the same three the package page reads them as.
export const HOSTS = ['cli', 'web', 'actor']

// A rate limiter can only count seconds, so the day's worth of new names is counted here instead.
export const MAX_NEW_NAMES = 10
const DAY = 86_400_000

/* A name that reads the same in a url, an import and a listing. */
export const named = (name: string) => name.length <= MAX_NAME && NAME.test(name) && !name.endsWith('-') && !name.includes('--')

export const versioned = (version: string) => VERSION.test(version)

export const described = (text: unknown) => text == null || (typeof text === 'string' && text.length <= MAX_DESCRIPTION)

export const linked = (url: unknown) =>
  url == null || (typeof url === 'string' && url.startsWith('https://') && url.length <= MAX_REPOSITORY && !/\s/.test(url))

// A LICENSE of any length is a notice, and the Apache one is eleven thousand characters.
export const noticed = (text: unknown) => text == null || (typeof text === 'string' && text.length <= MAX_NOTICE)

/* Where a program runs, as the artifact answers it, so a listing never has to open the bundle. */
export const hosted = (text: unknown): text is string =>
  typeof text === 'string' && text.length > 0 && text.split(',').every((host) => HOSTS.includes(host))

/* The pages a bundle carried, held to the same convention the CLI checked before packing, since a token holder can still post by hand. */
export function pages(raw: unknown): Page[] {
  if (raw == null || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Send the doc pages as an object.')

  const entries = Object.entries(raw as Record<string, unknown>)
  if (entries.length > MAX_PAGES) throw new Error(`A package carries ${MAX_PAGES} doc pages at most.`)

  return entries.map(([path, body]) => {
    if (typeof body !== 'string' || body.length > MAX_PAGE) throw new Error(`'${path}' is not a page of ${MAX_PAGE} bytes or fewer.`)

    check(path, body)
    return { path, body }
  })
}

/* Where a published artifact lives, the same path a consumer's imports entry points at. */
export const keyOf = (name: string, version: string) => `pkg/${name}/${version}/app.edge`

/* How many names this account claimed today, which is the scarce thing a squatter wants. */
export const claimedToday = async (db: D1Database, userId: string) =>
  ((await db
    .prepare('select count(*) as taken from package where user_id = ? and created_at > ?')
    .bind(userId, Date.now() - DAY)
    .first<{ taken: number }>())?.taken ?? 0)

export const packageByName = (db: D1Database, name: string) =>
  db.prepare('select * from package where name = ?').bind(name).first<Package>()

export const versionExists = async (db: D1Database, name: string, version: string) =>
  Boolean(await db.prepare('select 1 from version where package = ? and version = ?').bind(name, version).first())

/* Claims the name when it is free and records the version with its pages, all of it or none. */
export async function publish(db: D1Database, userId: string, release: Release) {
  const { name, version, digest, size, description, repository, notice, hosts, docs } = release
  const now = Date.now()

  await db.batch([
    db.prepare('insert or ignore into package (name, user_id, created_at) values (?, ?, ?)').bind(name, userId, now),
    db
      .prepare(
        'insert into version (package, version, digest, size, description, repository, license, notice, hosts, published_at) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)'
      )
      .bind(name, version, digest, size, description, repository, notice && identify(notice), notice, hosts, now),
    ...docs.map((page) =>
      db.prepare('insert into doc (package, version, path, body) values (?, ?, ?, ?)').bind(name, version, page.path, page.body)
    )
  ])
}

export const versionsOf = (db: D1Database, name: string) =>
  db
    .prepare('select version, digest, size, published_at, yanked_at from version where package = ? order by published_at desc')
    .bind(name)
    .all<{ version: string; digest: string; size: number; published_at: number; yanked_at: number | null }>()
