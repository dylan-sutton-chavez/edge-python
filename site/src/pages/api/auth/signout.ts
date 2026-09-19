import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { endSession } from '../../../lib/server/session'
import { json } from '../../../lib/server/http'

export const POST: APIRoute = async ({ cookies }) => {
  await endSession(env.DB, cookies)

  return json({ ok: true })
}
