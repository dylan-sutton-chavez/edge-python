import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { deleteUser, handleTaken, handleWait, publicUser, updateProfile, userById } from '../../lib/server/users'
import { endSession } from '../../lib/server/session'
import { spend } from '../../lib/server/otp'
import { body, json } from '../../lib/server/http'
import { validateHandle } from '../../lib/account/handle'
import { ICONS, PALETTES, type Palette } from '../../lib/account/avatar'

const plural = (count: number, unit: string) => `${count} ${unit}${count === 1 ? '' : 's'}`

const inWords = (left: number) => {
  const hours = Math.ceil(left / 3_600_000)
  return hours >= 24 ? plural(Math.ceil(hours / 24), 'day') : plural(hours, 'hour')
}

export const PATCH: APIRoute = async ({ request, locals }) => {
  const { user } = locals
  if (!user) return json({ error: 'Not signed in.' }, 401)

  const { handle, name, avatar, bio } = await body<{ handle: string; name: string; avatar: { icon: number; palette: Palette }; bio: string }>(request)

  const problem = typeof handle === 'string' ? validateHandle(handle) : 'Pick a handle.'
  if (problem) return json({ error: problem }, 400)
  if (await handleTaken(env.DB, handle!, user.id)) return json({ error: `@${handle} is already taken.` }, 409)

  const icon = Number(avatar?.icon)
  if (!Number.isInteger(icon) || icon < 1 || icon > ICONS || !PALETTES.includes(avatar?.palette as Palette)) return json({ error: 'Pick an avatar.' }, 400)

  // Naming yourself for the first time is free, moving a handle you already hold starts the wait.
  const current = await userById(env.DB, user.id)
  const moved = Boolean(current?.handle) && current!.handle !== handle

  if (moved) {
    const left = handleWait(current!.handle_changed_at)
    if (left > 0) return json({ error: `You can change your handle again in ${inWords(left)}.` }, 429)
  }

  const about = typeof bio === 'string' ? bio.trim().slice(0, 160) : undefined

  return json(publicUser(await updateProfile(env.DB, user.id, { handle: handle!, name: String(name ?? '').trim().slice(0, 64), avatar: { icon, palette: avatar!.palette }, bio: about }, moved)))
}

// Nothing here comes back, so a live session is not enough. A fresh code proves the mailbox is still yours.
export const DELETE: APIRoute = async ({ request, locals, cookies }) => {
  const { user } = locals
  if (!user) return json({ error: 'Not signed in.' }, 401)

  const { code } = await body<{ code: string }>(request)
  if (!user.email) return json({ error: 'This account has no address to confirm with.' }, 409)
  if (!(await spend(env.DB, cookies, user.email, String(code ?? '')))) return json({ error: 'That code is wrong or expired.' }, 403)

  await deleteUser(env.DB, user.id)
  await endSession(env.DB, cookies)

  return json({ ok: true })
}
