import type { APIRoute } from 'astro'
import { getCollection } from 'astro:content'
import { tree } from '../lib/docs/tree'

// Profiles and packages stay out until they carry enough of their own to be worth a visit.
export const GET: APIRoute = async ({ site, url }) => {
  const origin = (site ?? url).origin
  const docs = tree(await getCollection('docs'), '').flatMap((section) => section.docs)
  const paths = ['/', ...docs.map((doc) => `/docs/${doc.slug}`)]

  const body = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${paths.map((path) => `  <url><loc>${origin}${path}</loc></url>`).join('\n')}
</urlset>
`

  return new Response(body, { headers: { 'content-type': 'application/xml' } })
}
