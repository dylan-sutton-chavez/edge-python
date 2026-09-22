import type { AstroCookies } from 'astro'
import { random, sha256 } from '../crypto'
import type { Holder } from './users'

const COOKIE = '__Host-session'
const DAY = 86_400_000
const LIFETIME = 30 * DAY

// Opening one sweeps the expired, so the table holds only what is still live.
export async function startSession(db: D1Database, cookies: AstroCookies, userId: string) {
  const token = random()
  const now = Date.now()
  const expires = now + LIFETIME

  await db.batch([
    db.prepare('delete from session where expires_at < ?').bind(now),
    db.prepare('insert into session (token_hash, user_id, created_at, expires_at) values (?, ?, ?, ?)').bind(await sha256(token), userId, now, expires)
  ])

  cookies.set(COOKIE, token, { path: '/', httpOnly: true, secure: true, sameSite: 'lax', expires: new Date(expires) })
}

/* The signed-in user with the address its codes go to, which lives in account rather than on the row. */
export async function readSession(db: D1Database, cookies: AstroCookies) {
  const token = cookies.get(COOKIE)?.value
  if (!token) return null

  return db
    .prepare(
      `select user.*, address.provider_id as email
       from session
       join user on user.id = session.user_id
       left join account as address on address.user_id = user.id and address.provider = 'email'
       where session.token_hash = ? and session.expires_at > ?`
    )
    .bind(await sha256(token), Date.now())
    .first<Holder>()
}

export async function endSession(db: D1Database, cookies: AstroCookies) {
  const token = cookies.get(COOKIE)?.value
  if (token) await db.prepare('delete from session where token_hash = ?').bind(await sha256(token)).run()
  cookies.delete(COOKIE, { path: '/' })
}
