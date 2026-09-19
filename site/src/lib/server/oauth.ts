import { env } from 'cloudflare:workers'
import { base64url } from './crypto'
import type { Identity } from './users'

export type Provider = 'github' | 'google'

const ENDPOINTS = {
  github: { authorize: 'https://github.com/login/oauth/authorize', token: 'https://github.com/login/oauth/access_token', scope: 'read:user user:email' },
  google: { authorize: 'https://accounts.google.com/o/oauth2/v2/auth', token: 'https://oauth2.googleapis.com/token', scope: 'openid email profile' }
}

export const isProvider = (value: string | undefined): value is Provider => value === 'github' || value === 'google'

const credentials = (provider: Provider) =>
  provider === 'github'
    ? { id: env.OAUTH_GITHUB_ID, secret: env.OAUTH_GITHUB_SECRET }
    : { id: env.OAUTH_GOOGLE_ID, secret: env.OAUTH_GOOGLE_SECRET }

export const isConfigured = (provider: Provider) => Boolean(credentials(provider).id && credentials(provider).secret)

const callback = (provider: Provider) => `${env.SITE}/api/auth/callback/${provider}`

export async function authorizeUrl(provider: Provider, state: string, verifier: string) {
  const url = new URL(ENDPOINTS[provider].authorize)
  url.searchParams.set('client_id', credentials(provider).id)
  url.searchParams.set('redirect_uri', callback(provider))
  url.searchParams.set('scope', ENDPOINTS[provider].scope)
  url.searchParams.set('state', state)

  if (provider === 'google') {
    url.searchParams.set('response_type', 'code')
    url.searchParams.set('code_challenge', base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)))))
    url.searchParams.set('code_challenge_method', 'S256')
  }

  return url
}

export async function exchange(provider: Provider, code: string, verifier: string): Promise<Identity> {
  const { id, secret } = credentials(provider)
  const form = new URLSearchParams({ client_id: id, client_secret: secret, code, redirect_uri: callback(provider), grant_type: 'authorization_code' })
  if (provider === 'google') form.set('code_verifier', verifier)

  const response = await fetch(ENDPOINTS[provider].token, { method: 'POST', headers: { accept: 'application/json' }, body: form })
  if (!response.ok) throw new Error(`${provider} token exchange failed with ${response.status}.`)

  const tokens = (await response.json()) as { access_token?: string; id_token?: string; error?: string }
  if (tokens.error) throw new Error(`${provider} token exchange failed: ${tokens.error}.`)

  return provider === 'google' ? fromGoogle(tokens.id_token!) : fromGitHub(tokens.access_token!)
}

function fromGoogle(idToken: string): Identity {
  const payload = JSON.parse(atob(idToken.split('.')[1]!.replaceAll('-', '+').replaceAll('_', '/'))) as { sub: string; email: string; email_verified: boolean; name?: string }
  if (!payload.email_verified) throw new Error('Google account email is not verified.')

  return { provider: 'google', providerId: payload.sub, email: payload.email.toLowerCase(), name: payload.name }
}

async function fromGitHub(token: string): Promise<Identity> {
  const headers = { authorization: `Bearer ${token}`, accept: 'application/vnd.github+json', 'user-agent': 'edge-python-registry' }
  const [user, emails] = await Promise.all([
    fetch('https://api.github.com/user', { headers }).then((r) => r.json() as Promise<{ id: number; login: string; name: string | null }>),
    fetch('https://api.github.com/user/emails', { headers }).then((r) => r.json() as Promise<{ email: string; primary: boolean; verified: boolean }[]>)
  ])

  const email = emails.find((each) => each.primary && each.verified) ?? emails.find((each) => each.verified)
  if (!email) throw new Error('GitHub account has no verified email.')

  return { provider: 'github', providerId: String(user.id), email: email.email.toLowerCase(), name: user.name ?? user.login }
}
