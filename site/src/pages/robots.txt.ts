import type { APIRoute } from 'astro'

// The api answers machines and settings answers nobody signed out, neither belongs in an index.
export const GET: APIRoute = ({ site, url }) => {
  const origin = (site ?? url).origin

  const body = `User-agent: *
Disallow: /api/
Disallow: /settings

Sitemap: ${origin}/sitemap.xml
`

  return new Response(body, { headers: { 'content-type': 'text/plain' } })
}
