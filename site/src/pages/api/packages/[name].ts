import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { json } from '../../../lib/server/http'
import { keyOf, named, packageByName, versionsOf } from '../../../lib/server/packages'

// What `edge add <name>` reads, so a manifest entry can carry the digest of the version it pinned.
export const GET: APIRoute = async ({ params }) => {
  const name = String(params.name ?? '').toLowerCase()
  if (!named(name)) return json({ error: 'No such package.' }, 404)

  const held = await packageByName(env.DB, name)
  if (!held) return json({ error: 'No such package.' }, 404)

  const { results } = await versionsOf(env.DB, name)
  const latest = results.find((each) => each.yanked_at === null)

  if (!latest) return json({ error: 'Every version of that package is yanked.' }, 410)

  return json({
    name,
    version: latest.version,
    digest: latest.digest,
    size: latest.size,
    // Null until something establishes it, so a consumer reads no claim rather than every host.
    hosts: latest.hosts,
    url: `${env.CDN}/${keyOf(name, latest.version)}`
  })
}
