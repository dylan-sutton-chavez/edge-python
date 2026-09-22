export function base64url(bytes: Uint8Array) {
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
}

export const random = (bytes = 32) => base64url(crypto.getRandomValues(new Uint8Array(bytes)))

export async function sha256(text: string) {
  return base64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))))
}

// Hex, because an integrity pin is written `#sha256-<64 hex chars>`.
export async function sha256hex(bytes: Uint8Array) {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as BufferSource))
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, '0')).join('')
}

export function equal(a: string, b: string) {
  if (a.length !== b.length) return false

  let diff = 0
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i)
  return diff === 0
}

export const digits = (count: number) => String(crypto.getRandomValues(new Uint32Array(1))[0]! % 10 ** count).padStart(count, '0')
