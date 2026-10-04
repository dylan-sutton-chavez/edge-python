import type { APIRoute } from 'astro'
import { cached, json } from '../../../lib/server/http'
import { programs } from '../../../data/programs'
import { hidden } from '../../../draft'

const CACHE_SECONDS = 60

// A program as the page at the same address describes it, missing while that page is a draft so one deploy never answers two ways.
export const GET: APIRoute = ({ params }) => {
  const slug = String(params.name ?? '').toLowerCase()
  const found = programs.find((each) => each.slug === slug)

  if (!found || hidden(`/program/${slug}`)) return json({ error: 'No such program.' }, 404)

  return cached(found, CACHE_SECONDS)
}
