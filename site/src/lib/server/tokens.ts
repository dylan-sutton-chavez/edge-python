import { equal, random, sha256 } from '../crypto'

export type Token = { id: string; name: string; created_at: number; used_at: number | null; expires_at: number | null }

// What the CLI publishes with. A dot never appears in base64url, so the id splits off exactly.
const PREFIX = 'edge_pat_'
const BEARER = 'Bearer '

// A replaced token keeps working for a day, long enough for a deploy to pick the new one up.
export const GRACE = 86_400_000

export const MAX_NAME = 40
export const MAX_TOKENS = 20

// base64url of 16 and 32 bytes, unpadded, the sizes the browser is meant to send.
const SALT = 22
const HASH = 43
const B64URL = /^[A-Za-z0-9_-]+$/

/* The one digest both sides compute, so the browser and the worker can never drift apart. */
export const digest = (salt: string, secret: string) => sha256(salt + secret)

/* A salt and hash the browser could have produced, checked before anything is stored. */
export const wellFormed = (salt: string, hash: string) =>
  salt.length === SALT && hash.length === HASH && B64URL.test(salt) && B64URL.test(hash)

/* Newest first, and never the salt or the hash. */
export const userTokens = (db: D1Database, userId: string) =>
  db
    .prepare('select id, name, created_at, used_at, expires_at from token where user_id = ? order by created_at desc')
    .bind(userId)
    .all<Token>()

/* The browser makes the secret, so what lands here is a hash of something the server never saw. Replacing puts the old token on a clock rather than deleting it, so a lost response never leaves anyone with nothing, and forgetting to revoke is safe because the clock runs out on its own. */
export async function createToken(db: D1Database, userId: string, name: string, salt: string, hash: string, replaces?: string) {
  const id = random(6)
  const now = Date.now()
  const stops = replaces ? now + GRACE : null

  // Writing one sweeps the replaced tokens whose day has run out.
  const writes = [
    db.prepare('delete from token where expires_at < ?').bind(now),
    db
      .prepare('insert into token (id, user_id, name, salt, hash, created_at) values (?, ?, ?, ?, ?, ?)')
      .bind(id, userId, name, salt, hash, now)
  ]

  // `expires_at is null` so replacing twice never pushes an already running clock further out.
  if (replaces) {
    writes.push(
      db
        .prepare('update token set expires_at = ? where id = ? and user_id = ? and expires_at is null')
        .bind(stops, replaces, userId)
    )
  }

  await db.batch(writes)

  return { id, stops }
}

export async function revokeToken(db: D1Database, userId: string, id: string) {
  const done = await db.prepare('delete from token where id = ? and user_id = ?').bind(id, userId).run()
  return done.meta.changes > 0
}

/* The user a publish belongs to, or null. Only publishing calls this, so a token never becomes a session. */
export async function tokenUser(db: D1Database, header: string | null) {
  const presented = header?.startsWith(BEARER) ? header.slice(BEARER.length) : null
  if (!presented?.startsWith(PREFIX)) return null

  const [id, secret] = presented.slice(PREFIX.length).split('.')
  if (!id || !secret) return null

  const now = Date.now()

  const row = await db
    .prepare('select user_id, salt, hash from token where id = ? and (expires_at is null or expires_at > ?)')
    .bind(id, now)
    .first<{ user_id: string; salt: string; hash: string }>()

  if (!row || !equal(await digest(row.salt, secret), row.hash)) return null

  await db.prepare('update token set used_at = ? where id = ?').bind(now, id).run()

  return row.user_id
}
