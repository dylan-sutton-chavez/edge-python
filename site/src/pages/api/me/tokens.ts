import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { body, json } from '../../../lib/server/http'
import { createToken, userTokens, wellFormed, MAX_NAME, MAX_TOKENS } from '../../../lib/server/tokens'

// The browser sends a salt and a hash, never the secret, so there is nothing here to hand back.
export const POST: APIRoute = async ({ request, locals }) => {
  if (!locals.user) return json({ error: 'Not signed in.' }, 401)

  const { name, salt, hash, replaces } = await body<{ name: string; salt: string; hash: string; replaces: string }>(request)
  const trimmed = name?.trim()

  if (!trimmed || trimmed.length > MAX_NAME) return json({ error: `Name it in ${MAX_NAME} characters or fewer.` }, 400)
  if (!salt || !hash || !wellFormed(salt, hash)) return json({ error: 'That token was not generated here.' }, 400)

  const held = await userTokens(env.DB, locals.user.id)

  // A replacement is net zero once the old token's day runs out, so only a plain create meets the cap.
  if (!replaces && held.results.length >= MAX_TOKENS) return json({ error: `You can hold ${MAX_TOKENS} tokens at a time.` }, 409)
  if (replaces && !held.results.some((token) => token.id === replaces)) return json({ error: 'No such token.' }, 404)

  return json({ id: await createToken(env.DB, locals.user.id, trimmed, salt, hash, replaces) }, 201)
}
