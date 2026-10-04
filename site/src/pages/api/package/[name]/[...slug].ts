import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { cached, json, tooMany } from '../../../../lib/server/http'
import { keyOf, packageByName, versionsOf } from '../../../../lib/server/packages'
import { userById } from '../../../../lib/server/users'
import { packed } from '../../../../lib/server/bundle'
import { identify } from '../../../../lib/server/license'
import { front } from '../../../../lib/docs/convention'
import { entries } from '../../../../lib/docs/page'
import { tree } from '../../../../lib/docs/tree'

// Nothing here counts, so one answer serves everybody until a version is published.
const CACHE_SECONDS = 60

// What the page at the same address shows, without the markup or the counting the resolver holds.
export const GET: APIRoute = async ({ params, url, request }) => {
  if (await tooMany(env.READ_IP, request)) return json({ error: 'Too many requests. Try again later.' }, 429)

  const name = String(params.name ?? '').toLowerCase()
  const held = await packageByName(env.DB, name)
  if (!held) return json({ error: 'No such package.' }, 404)

  const { results } = await versionsOf(env.DB, name)
  const releases = results.filter((each) => each.yanked_at === null)

  // A ?v= asks for that version, otherwise the newest one still live.
  const asked = url.searchParams.get('v')
  const release = asked ? releases.find((each) => each.version === asked) : releases[0]

  if (!release) return json({ error: asked ? `${name} has no live version ${asked}.` : 'Every version of that package is yanked.' }, 404)

  const object = await env.CDN_BUCKET.get(keyOf(name, release.version))
  if (!object) return json({ error: 'No such package.' }, 404)

  const artifact = packed(new Uint8Array(await object.arrayBuffer()))
  const docs = tree(entries(artifact.docs), '').flatMap((section) => section.docs)

  // A slug asks for one page, and its markdown is the answer since the frontmatter only named it.
  if (params.slug) {
    const doc = docs.find((each) => each.slug === params.slug)
    if (!doc) return json({ error: `${name} has no page '${params.slug}'.` }, 404)

    const { body } = front(doc.id, artifact.docs[doc.id]!)
    return cached({ name, version: release.version, slug: doc.slug, title: doc.title, body }, CACHE_SECONDS)
  }

  const author = await userById(env.DB, held.user_id)

  return cached(
    {
      name,
      version: release.version,
      description: typeof artifact.description === 'string' ? artifact.description : null,
      repository: typeof artifact.repository === 'string' ? artifact.repository : null,
      license: artifact.notice && identify(artifact.notice),
      edge: typeof artifact.edge === 'string' ? artifact.edge : null,
      downloads: held.downloads,
      handle: author?.handle ?? null,
      versions: releases.map((each) => ({ version: each.version, size: each.size, published_at: each.published_at })),
      docs: docs.map((each) => ({ slug: each.slug, title: each.title }))
    },
    CACHE_SECONDS
  )
}
