import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { handleTaken } from '../../../lib/server/users'
import { json } from '../../../lib/server/http'
import { validateHandle } from '../../../lib/account/handle'

export const GET: APIRoute = async ({ params, locals }) => {
  const handle = params.handle ?? ''

  return json({ available: !validateHandle(handle) && !(await handleTaken(env.DB, handle, locals.user?.id)) })
}
