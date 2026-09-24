import { check } from '../docs/convention'
import { parts } from '../docs/sections'
import { identify } from './license'

export type Package = { name: string; user_id: string; downloads: number; created_at: number }

export type Release = {
  name: string
  version: string
  digest: string
  size: number
  description: string | null
  notice: string | null
  pages: Page[]
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

// A page as the search index holds it, the body without its frontmatter since nobody searches for a title twice.
export type Page = { path: string; title: string; body: string }

/* Holds the pages a bundle carried to the same convention the CLI checked before packing, since a token holder can still post by hand and a page the renderer cannot lay out belongs nowhere. What it read comes back, because the index wants the same title the check demanded. */
export function checkPages(raw: unknown): Page[] {
  if (raw == null || typeof raw !== 'object' || Array.isArray(raw)) throw new Error('Send the doc pages as an object.')

  const entries = Object.entries(raw as Record<string, unknown>)
  if (entries.length > MAX_PAGES) throw new Error(`A package carries ${MAX_PAGES} doc pages at most.`)

  return entries.map(([path, body]) => {
    if (typeof body !== 'string' || body.length > MAX_PAGE) throw new Error(`'${path}' is not a page of ${MAX_PAGE} bytes or fewer.`)

    const read = check(path, body)
    return { path, title: read.keys.get('title') ?? path, body: read.body }
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

/* Claims the name when it is free and records the version, all of it or none. The pages go to the index too, replacing whatever the last version left, since searching an old release's docs only finds pages nobody can reach. */
export async function publish(db: D1Database, userId: string, release: Release) {
  const { name, version, digest, size, description, notice, pages } = release
  const now = Date.now()

  await db.batch([
    db.prepare('insert or ignore into package (name, user_id, created_at) values (?, ?, ?)').bind(name, userId, now),
    db
      .prepare('insert into version (package, version, digest, size, description, license, published_at) values (?, ?, ?, ?, ?, ?, ?)')
      .bind(name, version, digest, size, description, notice && identify(notice), now),
    db.prepare('delete from page_search where package = ?').bind(name),
    ...pages.flatMap((page) =>
      parts(page.body).map((part) =>
        db
          .prepare('insert into page_search (package, path, title, section, anchor, body) values (?, ?, ?, ?, ?, ?)')
          .bind(name, page.path, page.title, part.section, part.anchor, part.body)
      )
    )
  ])
}

/* A package as a listing shows it, its newest live version describing it and its author beside it. A handle narrows the same query to one person's shelf. */
export type Listed = {
  name: string
  description: string | null
  license: string | null
  downloads: number
  handle: string
  avatar_icon: number | null
  avatar_palette: string | null
}

/* The same listing, narrowed. A handle holds it to one person's shelf, and a query reaches the name, what it says about itself, the license it carries and the prose of its own documentation, which is everything a package tells the registry about itself. */
export function listed(db: D1Database, { handle, asked, limit = 60 }: { handle?: string; asked?: string; limit?: number } = {}) {
  const held = handle ? 'and u.handle = ?' : ''
  const like = asked ? `%${asked}%` : ''

  const matching = asked
    ? `and (p.name like ?
            or v.description like ?
            or v.license like ?
            or u.handle like ?
            or exists (select 1 from page_search where package = p.name and page_search match ?))`
    : ''

  return db
    .prepare(
      `select p.name, p.downloads, v.description, v.license, u.handle, u.avatar_icon, u.avatar_palette
       from package p
         join version v on v.package = p.name
         join user u on u.id = p.user_id
       where v.published_at = (select max(published_at) from version where package = p.name and yanked_at is null)
         ${held}
         ${matching}
       order by ${asked ? 'case when p.name like ? then 0 else 1 end, ' : ''}p.downloads desc, p.name
       limit ?`
    )
    .bind(
      ...(handle ? [handle] : []),
      ...(asked ? [like, like, like, like, phrased(asked), `${asked}%`] : []),
      limit
    )
    .all<Listed>()
}

/* One more reach for this package, counted where `edge add` asks for a digest, since that is the moment somebody puts it in a project rather than merely reads its page. */
export const downloaded = (db: D1Database, name: string) =>
  db.prepare('update package set downloads = downloads + 1 where name = ?').bind(name).run()

export const counted = async (db: D1Database) =>
  ((await db
    .prepare('select count(distinct package) as total from version where yanked_at is null')
    .first<{ total: number }>())?.total ?? 0)

// A hit inside a package's documentation, carrying enough to draw a row without a second request.
export type Hit = { package: string; path: string; title: string; section: string; anchor: string; snippet: string }

/* What brackets the matched run inside a snippet. Control characters, not tags, because the body they surround is markdown from a stranger and a client that reads them builds text nodes rather than markup. */
export const MARK = { open: '\u0001', close: '\u0002' }

// Trigrams take the query as one phrase, so a quote inside it would end the phrase early.
const phrased = (asked: string) => `"${asked.replaceAll('"', '""')}"`

/* Where a query lands inside the published documentation, one row per section so a result opens at the words rather than at the top of the page. A heading outranks a title and both outrank the prose, since someone typing `receive` wants the section about it before a page that mentions it once. */
export const searched = (db: D1Database, asked: string, limit = 6) =>
  db
    .prepare(
      `select package, path, title, section, anchor, snippet(page_search, 5, ?, ?, '…', 14) as snippet
       from page_search
       where page_search match ?
       order by bm25(page_search, 0.0, 0.0, 6.0, 10.0, 0.0, 1.0)
       limit ?`
    )
    .bind(MARK.open, MARK.close, phrased(asked), limit)
    .all<Hit>()

export const versionsOf = (db: D1Database, name: string) =>
  db
    .prepare('select version, digest, size, hosts, published_at, yanked_at from version where package = ? order by published_at desc')
    .bind(name)
    .all<{ version: string; digest: string; size: number; hosts: string | null; published_at: number; yanked_at: number | null }>()
