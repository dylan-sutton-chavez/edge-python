import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { digits, random, sha256 } from '../../../../lib/server/crypto'
import { body, ip, json } from '../../../../lib/server/http'
import { sendCodeMail } from '../../../../lib/server/mailer'

const MINUTE = 60_000
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

export const POST: APIRoute = async (context) => {
  const { email: raw } = await body<{ email: string }>(context.request)
  const email = String(raw ?? '').trim().toLowerCase()
  if (!EMAIL.test(email)) return json({ error: 'Enter a valid email.' }, 400)

  const [byIp, byEmail] = await Promise.all([env.OTP_IP.limit({ key: ip(context) }), env.OTP_EMAIL.limit({ key: email })])
  if (!byIp.success || !byEmail.success) return json({ error: 'Too many codes requested. Try again in a minute.' }, 429)

  const code = digits(6)
  const nonce = random()

  await env.DB.prepare('insert or replace into email_code (email, hash, expires_at, attempts) values (?, ?, ?, 0)')
    .bind(email, await sha256(`${nonce}:${email}:${code}`), Date.now() + 10 * MINUTE)
    .run()

  context.cookies.set('__Host-otp', nonce, { path: '/', httpOnly: true, secure: true, sameSite: 'lax', maxAge: 600 })
  await sendCodeMail(email, code)

  return json({ ok: true })
}
