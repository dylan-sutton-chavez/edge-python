import type { APIRoute } from 'astro'
import { getCollection } from 'astro:content'
import { cached, json } from '../../../lib/server/http'
import { tree } from '../../../lib/docs/tree'

// The pages ship inside this worker, so an answer only changes when a deploy does.
const CACHE_SECONDS = 60

// What the page at the same address lays out, as markdown, and the whole index when no slug names one.
export const GET: APIRoute = async ({ params }) => {
  const entries = await getCollection('docs')
  const docs = tree(entries, '').flatMap((section) => section.docs)

  if (!params.slug) return cached({ docs: docs.map((each) => ({ slug: each.slug, title: each.title })) }, CACHE_SECONDS)

  const doc = docs.find((each) => each.slug === params.slug)
  if (!doc) return json({ error: 'No such page.' }, 404)

  const entry = entries.find((each) => each.id === doc.id)!

  return cached({ slug: doc.slug, title: doc.title, description: entry.data.description ?? null, body: entry.body ?? '' }, CACHE_SECONDS)
}
