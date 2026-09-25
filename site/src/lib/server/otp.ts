import type { AstroCookies } from 'astro'
import { digits, equal, random, sha256 } from '../crypto'
import { CODE_LIFE, type Purpose } from '../otp'
import { sendCodeMail } from './mailer'

const COOKIE = '__Host-otp'
const ATTEMPTS = 5

// The purpose is hashed with the code, so one mailed for signing in cannot delete an account.
const fingerprint = (nonce: string, purpose: Purpose, email: string, code: string) => sha256(`${nonce}:${purpose}:${email}:${code}`)

/* Whether `email` already holds a live code for `purpose`, which a reopened dialog returns to instead of mailing another. */
export async function live(db: D1Database, email: string, purpose: Purpose) {
  const row = await db
    .prepare('select purpose from email_code where email = ? and expires_at > ?')
    .bind(email, Date.now())
    .first<{ purpose: Purpose }>()

  return row?.purpose === purpose
}

/* Mails a code for `purpose`, replacing whatever the address held, since one address holds one code. Callers ask `live` first so a reused code costs nothing. */
export async function issue(db: D1Database, cookies: AstroCookies, email: string, purpose: Purpose) {
  const now = Date.now()
  const code = digits(6)
  const nonce = random()

  // Writing one sweeps the expired, so codes nobody came back for cannot pile up.
  await db.batch([
    db.prepare('delete from email_code where expires_at < ?').bind(now),
    db
      .prepare('insert or replace into email_code (email, purpose, hash, attempts, created_at, expires_at) values (?, ?, ?, 0, ?, ?)')
      .bind(email, purpose, await fingerprint(nonce, purpose, email, code), now, now + CODE_LIFE)
  ])

  cookies.set(COOKIE, nonce, { path: '/', httpOnly: true, secure: true, sameSite: 'lax', maxAge: CODE_LIFE / 1000 })
  await sendCodeMail(email, code, purpose)
}

/* Spends the code mailed to `email` for `purpose`, true only once. The nonce comes from the browser that asked for it, so a code read out to someone else is useless anywhere but here. */
export async function spend(db: D1Database, cookies: AstroCookies, email: string, purpose: Purpose, code: string) {
  const nonce = cookies.get(COOKIE)?.value

  const row = await db
    .prepare('select purpose, hash, expires_at, attempts from email_code where email = ?')
    .bind(email)
    .first<{ purpose: Purpose; hash: string; expires_at: number; attempts: number }>()

  if (!nonce || !row || row.purpose !== purpose || row.expires_at < Date.now() || row.attempts >= ATTEMPTS) return false

  if (!equal(row.hash, await fingerprint(nonce, purpose, email, code.trim()))) {
    await db.prepare('update email_code set attempts = attempts + 1 where email = ?').bind(email).run()
    return false
  }

  await db.prepare('delete from email_code where email = ?').bind(email).run()
  cookies.delete(COOKIE, { path: '/' })

  return true
}
