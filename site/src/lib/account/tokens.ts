import { random, sha256 } from '../crypto'

const PREFIX = 'edge_pat_'

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

/* The secret is born here and only its hash leaves, so no server sees it even once. Pass `replaces` to make this a replacement, and `stops` comes back as the moment the old token gives out. */
export async function createToken(name: string, replaces?: string) {
  const secret = random(32)
  const salt = random(16)
  const hash = await sha256(salt + secret)

  const { id, stops } = await call<{ id: string; stops: number | null }>('/api/me/tokens', 'POST', { name, salt, hash, replaces })

  return { id, stops, token: `${PREFIX}${id}.${secret}` }
}

export const revokeToken = (id: string) => call<{ ok: boolean }>(`/api/me/tokens/${id}`, 'DELETE')
