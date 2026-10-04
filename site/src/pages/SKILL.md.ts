import type { APIRoute } from 'astro'
import skill from '../../../skill/SKILL.md?raw'

// The reference the briefing sends an agent to, served from the origin that answers its other questions.
export const GET: APIRoute = () =>
  new Response(skill, { headers: { 'content-type': 'text/markdown; charset=utf-8', 'cache-control': 'public, max-age=60', 'x-robots-tag': 'noindex' } })
