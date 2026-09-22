import type { AstroCookies } from 'astro'
import { equal, sha256 } from '../crypto'

const COOKIE = '__Host-otp'
const ATTEMPTS = 5

/* Spends the code mailed to `email`, true only once. The nonce comes from the browser that asked for it, so a code read out to someone else is useless anywhere but here. */
export async function spend(db: D1Database, cookies: AstroCookies, email: string, code: string) {
  const nonce = cookies.get(COOKIE)?.value

  const row = await db
    .prepare('select hash, expires_at, attempts from email_code where email = ?')
    .bind(email)
    .first<{ hash: string; expires_at: number; attempts: number }>()

  if (!nonce || !row || row.expires_at < Date.now() || row.attempts >= ATTEMPTS) return false

  if (!equal(row.hash, await sha256(`${nonce}:${email}:${code.trim()}`))) {
    await db.prepare('update email_code set attempts = attempts + 1 where email = ?').bind(email).run()
    return false
  }

  await db.prepare('delete from email_code where email = ?').bind(email).run()
  cookies.delete(COOKIE, { path: '/' })

  return true
}
