import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { isProvider } from '../../../../lib/server/oauth'
import { unlinkAccount } from '../../../../lib/server/users'
import { json } from '../../../../lib/server/http'

// The email code always works, so any OAuth provider can be dropped without locking the user out.
export const DELETE: APIRoute = async ({ params, locals }) => {
  if (!locals.user) return json({ error: 'Not signed in.' }, 401)
  if (!isProvider(params.provider)) return json({ error: 'Unknown provider.' }, 404)

  await unlinkAccount(env.DB, locals.user.id, params.provider)

  return json({ ok: true })
}
