import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { equal, sha256 } from '../../../../lib/server/crypto'
import { body, json } from '../../../../lib/server/http'
import { upsertUser } from '../../../../lib/server/users'
import { startSession } from '../../../../lib/server/session'

const ATTEMPTS = 5

export const POST: APIRoute = async ({ request, cookies }) => {
  const { email: raw, code } = await body<{ email: string; code: string }>(request)
  const email = String(raw ?? '').trim().toLowerCase()
  const nonce = cookies.get('__Host-otp')?.value
  const row = await env.DB.prepare('select hash, expires_at, attempts from email_code where email = ?').bind(email).first<{ hash: string; expires_at: number; attempts: number }>()

  if (!nonce || !row || row.expires_at < Date.now() || row.attempts >= ATTEMPTS) return json({ ok: false, handle: null })

  if (!equal(row.hash, await sha256(`${nonce}:${email}:${String(code ?? '').trim()}`))) {
    await env.DB.prepare('update email_code set attempts = attempts + 1 where email = ?').bind(email).run()
    return json({ ok: false, handle: null })
  }

  await env.DB.prepare('delete from email_code where email = ?').bind(email).run()
  cookies.delete('__Host-otp', { path: '/' })

  const user = await upsertUser(env.DB, { provider: 'email', providerId: email, email })
  await startSession(env.DB, cookies, user.id)

  return json({ ok: true, handle: user.handle })
}
