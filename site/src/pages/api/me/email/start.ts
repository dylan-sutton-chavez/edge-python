import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { digits, random, sha256 } from '../../../../lib/crypto'
import { body, ip, json } from '../../../../lib/server/http'
import { sendCodeMail } from '../../../../lib/server/mailer'
import { addressTaken } from '../../../../lib/server/users'

const MINUTE = 60_000
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

// The code goes to the new address, so a typo cannot lock anyone out of an account they still hold.
export const POST: APIRoute = async (context) => {
  const { user } = context.locals
  if (!user) return json({ error: 'Not signed in.' }, 401)

  const { email: raw } = await body<{ email: string }>(context.request)
  const email = String(raw ?? '').trim().toLowerCase()

  if (!EMAIL.test(email)) return json({ error: 'Enter a valid email.' }, 400)
  if (email === user.email) return json({ error: 'That is already your address.' }, 409)
  if (await addressTaken(env.DB, email)) return json({ error: 'Another account already uses that address.' }, 409)

  const [byIp, byEmail] = await Promise.all([env.OTP_IP.limit({ key: ip(context) }), env.OTP_EMAIL.limit({ key: email })])
  if (!byIp.success || !byEmail.success) return json({ error: 'Too many codes requested. Try again in a minute.' }, 429)

  const code = digits(6)
  const nonce = random()
  const now = Date.now()

  await env.DB.batch([
    env.DB.prepare('delete from email_code where expires_at < ?').bind(now),
    env.DB
      .prepare('insert or replace into email_code (email, hash, attempts, created_at, expires_at) values (?, ?, 0, ?, ?)')
      .bind(email, await sha256(`${nonce}:${email}:${code}`), now, now + 10 * MINUTE)
  ])

  context.cookies.set('__Host-otp', nonce, { path: '/', httpOnly: true, secure: true, sameSite: 'lax', maxAge: 600 })
  await sendCodeMail(email, code)

  return json({ ok: true })
}
