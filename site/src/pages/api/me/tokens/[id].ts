import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { json } from '../../../../lib/server/http'
import { revokeToken } from '../../../../lib/server/tokens'

// The user filter is in the where clause, so an id from another account matches nothing rather than telling on itself.
export const DELETE: APIRoute = async ({ params, locals }) => {
  if (!locals.user) return json({ error: 'Not signed in.' }, 401)
  if (!params.id || !(await revokeToken(env.DB, locals.user.id, params.id))) return json({ error: 'No such token.' }, 404)

  return json({ ok: true })
}
