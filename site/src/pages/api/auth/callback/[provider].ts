import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { exchange, isProvider } from '../../../../lib/server/oauth'
import { linkAccount, upsertUser } from '../../../../lib/server/users'
import { startSession } from '../../../../lib/server/session'

export const GET: APIRoute = async ({ params, url, cookies, locals, redirect }) => {
  if (!isProvider(params.provider)) return new Response('Unknown provider', { status: 404 })

  const saved = JSON.parse(cookies.get('__Host-oauth')?.value ?? 'null') as { state: string; verifier: string } | null
  cookies.delete('__Host-oauth', { path: '/' })

  const code = url.searchParams.get('code')
  if (!saved || !code || url.searchParams.get('state') !== saved.state) return new Response('Invalid state', { status: 400 })

  const identity = await exchange(params.provider, code, saved.verifier)

  if (locals.user) {
    await linkAccount(env.DB, locals.user.id, identity)
    return redirect('/settings#account')
  }

  const user = await upsertUser(env.DB, identity)
  await startSession(env.DB, cookies, user.id)

  return redirect(user.handle ? '/' : '/?welcome')
}
