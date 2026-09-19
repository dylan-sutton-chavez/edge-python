import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { json } from '../../lib/server/http'

export const GET: APIRoute = async () => {
  const row = await env.DB.prepare('select count(*) as users from user').first<{ users: number }>()

  return json({ ok: true, users: row?.users ?? 0 })
}
