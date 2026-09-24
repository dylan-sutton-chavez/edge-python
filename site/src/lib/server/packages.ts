import { check } from '../docs/convention'

export type Package = { name: string; user_id: string; downloads: number; created_at: number }

export type Release = {
  name: string
  version: string
  digest: string
  size: number
  description: string | null
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

/* Holds the pages a bundle carried to the same convention the CLI checked before packing, since a token holder can still post by hand and a page the renderer cannot lay out belongs nowhere. */
export function checkPages(raw: unknown) {
  if (raw == null || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Send the doc pages as an object.')

  const entries = Object.entries(raw as Record<string, unknown>)
  if (entries.length > MAX_PAGES) throw new Error(`A package carries ${MAX_PAGES} doc pages at most.`)

  for (const [path, body] of entries) {
    if (typeof body !== 'string' || body.length > MAX_PAGE) throw new Error(`'${path}' is not a page of ${MAX_PAGE} bytes or fewer.`)

    check(path, body)
  }
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

/* Claims the name when it is free and records the version, all of it or none. */
export async function publish(db: D1Database, userId: string, release: Release) {
  const { name, version, digest, size, description } = release
  const now = Date.now()

  await db.batch([
    db.prepare('insert or ignore into package (name, user_id, created_at) values (?, ?, ?)').bind(name, userId, now),
    db
      .prepare('insert into version (package, version, digest, size, description, published_at) values (?, ?, ?, ?, ?, ?)')
      .bind(name, version, digest, size, description, now)
  ])
}

/* A package as a listing shows it, its newest live version describing it and its author beside it. A handle narrows the same query to one person's shelf. */
export type Listed = {
  name: string
  description: string | null
  downloads: number
  handle: string
  avatar_icon: number | null
  avatar_palette: string | null
}

export function listed(db: D1Database, { handle, limit = 60 }: { handle?: string; limit?: number } = {}) {
  const held = handle ? 'and u.handle = ?' : ''

  return db
    .prepare(
      `select p.name, p.downloads, v.description, u.handle, u.avatar_icon, u.avatar_palette
       from package p
         join version v on v.package = p.name
         join user u on u.id = p.user_id
       where v.published_at = (select max(published_at) from version where package = p.name and yanked_at is null)
         ${held}
       order by p.downloads desc, p.name
       limit ?`
    )
    .bind(...(handle ? [handle, limit] : [limit]))
    .all<Listed>()
}

/* One more reach for this package, counted where `edge add` asks for a digest, since that is the moment somebody puts it in a project rather than merely reads its page. */
export const downloaded = (db: D1Database, name: string) =>
  db.prepare('update package set downloads = downloads + 1 where name = ?').bind(name).run()

export const counted = async (db: D1Database) =>
  ((await db
    .prepare('select count(distinct package) as total from version where yanked_at is null')
    .first<{ total: number }>())?.total ?? 0)

export const versionsOf = (db: D1Database, name: string) =>
  db
    .prepare('select version, digest, size, hosts, published_at, yanked_at from version where package = ? order by published_at desc')
    .bind(name)
    .all<{ version: string; digest: string; size: number; hosts: string | null; published_at: number; yanked_at: number | null }>()
