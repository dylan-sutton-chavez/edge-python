import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { body, json } from '../../../../lib/server/http'
import { spend } from '../../../../lib/server/otp'
import { upsertUser } from '../../../../lib/server/users'
import { startSession } from '../../../../lib/server/session'

export const POST: APIRoute = async ({ request, cookies }) => {
  const { email: raw, code } = await body<{ email: string; code: string }>(request)
  const email = String(raw ?? '').trim().toLowerCase()

  if (!(await spend(env.DB, cookies, email, 'sign_in', String(code ?? '')))) return json({ ok: false, handle: null })

  const user = await upsertUser(env.DB, { provider: 'email', providerId: email, email })
  await startSession(env.DB, cookies, user.id)

  return json({ ok: true, handle: user.handle })
}
