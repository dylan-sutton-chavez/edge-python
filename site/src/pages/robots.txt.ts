import type { APIRoute } from 'astro'

// Settings stays crawlable, since a blocked page never shows the noindex that keeps it out.
export const GET: APIRoute = ({ site, url }) => {
  const origin = (site ?? url).origin

  const body = `User-agent: *
Disallow: /api/

Sitemap: ${origin}/sitemap.xml
`

  return new Response(body, { headers: { 'content-type': 'text/plain' } })
}
