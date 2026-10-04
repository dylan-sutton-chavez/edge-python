import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { cached, json, tooMany } from '../../lib/server/http'
import { listed } from '../../lib/server/packages'
import { publicUser, userByHandle } from '../../lib/server/users'

const CACHE_SECONDS = 60

// What the page at the same address shows, a person and everything they publish.
export const GET: APIRoute = async ({ params, request }) => {
  if (await tooMany(env.READ_IP, request)) return json({ error: 'Too many requests. Try again later.' }, 429)

  const found = await userByHandle(env.DB, params.user ?? '')
  if (!found) return json({ error: 'No such person.' }, 404)

  const user = publicUser(found)
  const { results } = await listed(env.DB, { handle: user.handle ?? '' })

  return cached(
    {
      handle: user.handle,
      name: user.name,
      bio: user.bio,
      packages: results.map((each) => ({ name: each.name, description: each.description, license: each.license, downloads: each.downloads }))
    },
    CACHE_SECONDS
  )
}
