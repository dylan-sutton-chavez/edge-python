import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { deleteUser, handleTaken, publicUser, updateProfile } from '../../lib/server/users'
import { endSession } from '../../lib/server/session'
import { body, json } from '../../lib/server/http'
import { validateHandle } from '../../lib/account/handle'
import { ICONS, PALETTES, type Palette } from '../../lib/account/avatar'

export const PATCH: APIRoute = async ({ request, locals }) => {
  const { user } = locals
  if (!user) return json({ error: 'Not signed in.' }, 401)

  const { handle, name, avatar, bio } = await body<{ handle: string; name: string; avatar: { icon: number; palette: Palette }; bio: string }>(request)

  const problem = typeof handle === 'string' ? validateHandle(handle) : 'Pick a handle.'
  if (problem) return json({ error: problem }, 400)
  if (await handleTaken(env.DB, handle!, user.id)) return json({ error: `@${handle} is already taken.` }, 409)

  const icon = Number(avatar?.icon)
  if (!Number.isInteger(icon) || icon < 1 || icon > ICONS || !PALETTES.includes(avatar?.palette as Palette)) return json({ error: 'Pick an avatar.' }, 400)

  const about = typeof bio === 'string' ? bio.trim().slice(0, 160) : undefined

  return json(publicUser(await updateProfile(env.DB, user.id, { handle: handle!, name: String(name ?? '').trim().slice(0, 64), avatar: { icon, palette: avatar!.palette }, bio: about })))
}

export const DELETE: APIRoute = async ({ locals, cookies }) => {
  if (!locals.user) return json({ error: 'Not signed in.' }, 401)

  await deleteUser(env.DB, locals.user.id)
  await endSession(env.DB, cookies)

  return json({ ok: true })
}
