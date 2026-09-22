export type Package = { name: string; user_id: string | null; created_at: number }

const NAME = /^[a-z][a-z0-9-]*$/
const VERSION = /^\d{1,9}\.\d{1,9}\.\d{1,9}$/

export const MAX_NAME = 40
export const MAX_ARTIFACT = 32 << 20

// A rate limiter can only count seconds, so the day's worth of new names is counted here instead.
export const MAX_NEW_NAMES = 10
const DAY = 86_400_000

/* A name that reads the same in a url, an import and a listing. */
export const named = (name: string) => name.length <= MAX_NAME && NAME.test(name) && !name.endsWith('-') && !name.includes('--')

export const versioned = (version: string) => VERSION.test(version)

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

/* Claims the name when it is free and records the version, both or neither. */
export async function publish(db: D1Database, userId: string, name: string, version: string, digest: string, size: number) {
  const now = Date.now()

  await db.batch([
    db.prepare('insert or ignore into package (name, user_id, created_at) values (?, ?, ?)').bind(name, userId, now),
    db
      .prepare('insert into version (package, version, digest, size, published_at) values (?, ?, ?, ?, ?)')
      .bind(name, version, digest, size, now)
  ])
}

export const versionsOf = (db: D1Database, name: string) =>
  db
    .prepare('select version, digest, size, published_at, yanked_at from version where package = ? order by published_at desc')
    .bind(name)
    .all<{ version: string; digest: string; size: number; published_at: number; yanked_at: number | null }>()
