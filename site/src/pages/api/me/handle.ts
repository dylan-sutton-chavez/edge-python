import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { handleTaken, vacatedBy } from '../../../lib/server/users'
import { json } from '../../../lib/server/http'
import { validateHandle } from '../../../lib/account/handle'

// Whether the form may claim this one, which is a question about signing up rather than about a person.
export const GET: APIRoute = async ({ url, locals }) => {
  const handle = url.searchParams.get('name') ?? ''
  if (validateHandle(handle) || (await handleTaken(env.DB, handle, locals.user?.id))) return json({ available: false })

  // One this visitor left is theirs to take back, anybody else's is held.
  const held = await vacatedBy(env.DB, handle)

  return json({ available: !held || held.left_by === locals.user?.id })
}
