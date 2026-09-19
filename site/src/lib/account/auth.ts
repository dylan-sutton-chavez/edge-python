import type { Avatar } from './avatar'

export type Profile = { handle: string; name: string; avatar: Avatar; bio?: string }
export type Me = { id: string; email: string; name: string | null; handle: string | null; bio: string | null; avatar: Avatar | null }

async function call<T>(path: string, method: string, data?: unknown): Promise<T> {
  const response = await fetch(path, {
    method,
    headers: data ? { 'content-type': 'application/json' } : undefined,
    body: data ? JSON.stringify(data) : undefined
  })

  if (!response.ok) {
    const problem = (await response.json().catch(() => null)) as { error?: string } | null
    throw new Error(problem?.error ?? `Request failed with ${response.status}.`)
  }

  return response.json() as Promise<T>
}

export const sendCode = (email: string) => call<{ ok: boolean }>('/api/auth/email/start', 'POST', { email })
export const verifyCode = (email: string, code: string) => call<{ ok: boolean; handle: string | null }>('/api/auth/email/verify', 'POST', { email, code })
export const checkHandle = (handle: string) => call<{ available: boolean }>(`/api/handles/${handle}`, 'GET').then((result) => result.available)
export const saveProfile = (profile: Profile) => call<Me>('/api/me', 'PATCH', profile)
export const signOut = () => call<{ ok: boolean }>('/api/auth/signout', 'POST')
export const deleteAccount = () => call<{ ok: boolean }>('/api/me', 'DELETE')
export const disconnect = (provider: string) => call<{ ok: boolean }>(`/api/me/accounts/${provider}`, 'DELETE')
