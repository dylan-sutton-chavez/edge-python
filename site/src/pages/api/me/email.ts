import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { body, json } from '../../../lib/server/http'
import { spend } from '../../../lib/server/otp'
import { sendMovedMail } from '../../../lib/server/mailer'
import { moveAddress } from '../../../lib/server/users'

export const PATCH: APIRoute = async ({ request, locals, cookies }) => {
  const { user } = locals
  if (!user) return json({ error: 'Not signed in.' }, 401)

  const { email: raw, code } = await body<{ email: string; code: string }>(request)
  const email = String(raw ?? '').trim().toLowerCase()

  // The code was mailed to this address, so spending it is the proof that the mailbox answers.
  if (!(await spend(env.DB, cookies, email, 'change_email', String(code ?? '')))) return json({ error: 'That code is wrong or expired.' }, 403)
  if (!(await moveAddress(env.DB, user.id, email))) return json({ error: 'Another account already uses that address.' }, 409)

  if (user.email) await sendMovedMail(user.email, email)

  return json({ email })
}
