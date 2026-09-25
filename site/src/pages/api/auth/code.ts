import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { body, ip, json } from '../../../lib/server/http'
import { PURPOSES, type Purpose } from '../../../lib/otp'
import { issue, live } from '../../../lib/server/otp'
import { addressTaken } from '../../../lib/server/users'

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

type Asked = { purpose: Purpose; email?: string; resend?: boolean }
type Holder = { email: string | null } | null | undefined
type Wanted = { email: string } | { error: string; status: 400 | 401 | 409 }

/* Which address a purpose mails and what has to hold first, kept together so the three differ where you can see it. */
async function wanted(purpose: Purpose, asked: string | undefined, user: Holder): Promise<Wanted> {
  // Ending an account reads its address off the session, never off the request, so nobody can aim it at a stranger.
  if (purpose === 'delete_account') {
    if (!user) return { error: 'Not signed in.', status: 401 }
    return user.email ? { email: user.email } : { error: 'This account has no address to confirm with.', status: 409 }
  }

  const typed = String(asked ?? '').trim().toLowerCase()
  if (!EMAIL.test(typed)) return { error: 'Enter a valid email.', status: 400 }

  // The code goes to the new address, so a typo cannot lock anyone out of an account they still hold.
  if (purpose === 'change_email') {
    if (!user) return { error: 'Not signed in.', status: 401 }
    if (typed === user.email) return { error: 'That is already your address.', status: 409 }
    if (await addressTaken(env.DB, typed)) return { error: 'Another account already uses that address.', status: 409 }
  }

  return { email: typed }
}

export const POST: APIRoute = async (context) => {
  const asked = await body<Asked>(context.request)
  const purpose = asked.purpose

  if (!purpose || !PURPOSES.includes(purpose)) return json({ error: 'Unknown code request.' }, 400)

  const target = await wanted(purpose, asked.email, context.locals.user)
  if ('error' in target) return json({ error: target.error }, target.status)

  // Reopening a dialog mails nothing, so it must not spend a rate limit token either.
  if (asked.resend !== true && (await live(env.DB, target.email, purpose))) return json({ ok: true, sent: false })

  const [byIp, byEmail] = await Promise.all([env.OTP_IP.limit({ key: ip(context) }), env.OTP_EMAIL.limit({ key: target.email })])
  if (!byIp.success || !byEmail.success) return json({ error: 'Too many codes requested. Try again in a minute.' }, 429)

  await issue(env.DB, context.cookies, target.email, purpose)

  return json({ ok: true, sent: true })
}
