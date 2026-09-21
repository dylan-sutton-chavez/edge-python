import type { AstroCookies } from 'astro'
import { random, sha256 } from '../crypto'
import type { User } from './users'

const COOKIE = '__Host-session'
const DAY = 86_400_000
const LIFETIME = 30 * DAY

export async function startSession(db: D1Database, cookies: AstroCookies, userId: string) {
  const token = random()
  const expires = Date.now() + LIFETIME

  await db.prepare('insert into session (id, user_id, expires_at) values (?, ?, ?)').bind(await sha256(token), userId, expires).run()
  cookies.set(COOKIE, token, { path: '/', httpOnly: true, secure: true, sameSite: 'lax', expires: new Date(expires) })
}

export async function readSession(db: D1Database, cookies: AstroCookies) {
  const token = cookies.get(COOKIE)?.value
  if (!token) return null

  return db
    .prepare('select user.* from session join user on user.id = session.user_id where session.id = ? and session.expires_at > ?')
    .bind(await sha256(token), Date.now())
    .first<User>()
}

export async function endSession(db: D1Database, cookies: AstroCookies) {
  const token = cookies.get(COOKIE)?.value
  if (token) await db.prepare('delete from session where id = ?').bind(await sha256(token)).run()
  cookies.delete(COOKIE, { path: '/' })
}
