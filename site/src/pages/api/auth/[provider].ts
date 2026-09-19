import type { APIRoute } from 'astro'
import { authorizeUrl, isConfigured, isProvider } from '../../../lib/server/oauth'
import { random } from '../../../lib/server/crypto'

export const GET: APIRoute = async ({ params, cookies, redirect }) => {
  if (!isProvider(params.provider)) return new Response('Unknown provider', { status: 404 })
  if (!isConfigured(params.provider)) return new Response(`Sign-in with ${params.provider} is not configured.`, { status: 503 })

  const state = random(16)
  const verifier = random()
  cookies.set('__Host-oauth', JSON.stringify({ state, verifier }), { path: '/', httpOnly: true, secure: true, sameSite: 'lax', maxAge: 600 })

  return redirect((await authorizeUrl(params.provider, state, verifier)).toString())
}
