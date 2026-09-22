import { random } from '../crypto'
import type { Avatar, Palette } from '../account/avatar'
import type { Me, Public } from '../account/auth'

export type User = {
  id: string
  email: string
  name: string | null
  handle: string | null
  avatar_icon: number | null
  avatar_palette: string | null
  bio: string | null
  created_at: number
  handle_changed_at: number | null
}

// A handle someone gives up goes back in the pool, so it cannot be traded away and reclaimed at will.
export const HANDLE_WAIT = 7 * 86_400_000

/* How long is left on the wait, or zero when the handle is free to change. */
export const handleWait = (changedAt: number | null) =>
  changedAt === null ? 0 : Math.max(0, changedAt + HANDLE_WAIT - Date.now())

export type Identity = { provider: 'github' | 'google' | 'email'; providerId: string; email: string; name?: string | null }

/* The user a credential belongs to, the one lookup every way in shares. */
export const byCredential = (db: D1Database, provider: string, providerId: string) =>
  db
    .prepare('select user.* from account join user on user.id = account.user_id where account.provider = ? and account.provider_id = ?')
    .bind(provider, providerId)
    .first<User>()

/* The user behind an identity, found by its own credential or by the address on file, and created when neither exists. An address lives only on the user row, so nothing here can drift from it. */
export async function upsertUser(db: D1Database, identity: Identity): Promise<User> {
  const own = await byCredential(db, identity.provider, identity.providerId)
  if (own) return own

  let user = await db.prepare('select * from user where email = ?').bind(identity.email).first<User>()

  if (!user) {
    const id = `u_${random(12)}`
    await db.prepare('insert into user (id, email, name, created_at) values (?, ?, ?, ?)').bind(id, identity.email, identity.name ?? null, Date.now()).run()
    user = (await userById(db, id))!
  }

  // A second Google or GitHub on one account is refused by the index, so an existing link stays put.
  if (identity.provider !== 'email') await linkAccount(db, user.id, identity)

  return user
}

export const handleTaken = async (db: D1Database, handle: string, except?: string) =>
  Boolean(await db.prepare('select 1 from user where handle = ? and id != ?').bind(handle, except ?? '').first())

/* Saves the profile, and starts the wait only when the handle actually moved. */
export async function updateProfile(db: D1Database, id: string, profile: { handle: string; name: string; avatar: Avatar; bio?: string }, moved: boolean) {
  await db
    .prepare('update user set handle = ?, name = ?, avatar_icon = ?, avatar_palette = ?, bio = coalesce(?, bio), handle_changed_at = coalesce(?, handle_changed_at) where id = ?')
    .bind(profile.handle, profile.name, profile.avatar.icon, profile.avatar.palette, profile.bio ?? null, moved ? Date.now() : null, id)
    .run()

  return (await db.prepare('select * from user where id = ?').bind(id).first<User>())!
}

export const publicUser = ({ id, name, handle, avatar_icon, avatar_palette, bio }: User): Public => ({
  id,
  name,
  handle,
  bio,
  avatar: avatar_icon && avatar_palette ? { icon: avatar_icon, palette: avatar_palette as Palette } : null
})

/* The signed-in view, which is the public one plus the address the codes go to. */
export const me = (user: User): Me => ({ ...publicUser(user), email: user.email })

export const userById = (db: D1Database, id: string) => db.prepare('select * from user where id = ?').bind(id).first<User>()

export const userByHandle = (db: D1Database, handle: string) => db.prepare('select * from user where handle = ?').bind(handle).first<User>()

export async function linkedProviders(db: D1Database, userId: string) {
  const { results } = await db.prepare('select provider from account where user_id = ?').bind(userId).all<{ provider: string }>()
  return results.map((row) => row.provider)
}

// Links an OAuth identity to the signed-in user, one already linked elsewhere stays where it is.
export async function linkAccount(db: D1Database, userId: string, identity: Identity) {
  await db.prepare('insert or ignore into account (provider, provider_id, user_id) values (?, ?, ?)').bind(identity.provider, identity.providerId, userId).run()
}

export async function unlinkAccount(db: D1Database, userId: string, provider: string) {
  await db.prepare('delete from account where user_id = ? and provider = ?').bind(userId, provider).run()
}

export async function deleteUser(db: D1Database, id: string) {
  await db.batch([
    db.prepare('delete from session where user_id = ?').bind(id),
    db.prepare('delete from account where user_id = ?').bind(id),
    db.prepare('delete from user where id = ?').bind(id)
  ])
}
