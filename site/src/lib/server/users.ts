import { random } from '../crypto'
import type { Avatar, Palette } from '../account/avatar'
import type { Me } from '../account/auth'

export type User = {
  id: string
  email: string
  name: string | null
  handle: string | null
  avatar_icon: number | null
  avatar_palette: string | null
  bio: string | null
  created_at: number
}

export type Identity = { provider: 'github' | 'google' | 'email'; providerId: string; email: string; name?: string | null }

export async function upsertUser(db: D1Database, identity: Identity): Promise<User> {
  const linked = await db
    .prepare('select user.* from account join user on user.id = account.user_id where account.provider = ? and account.provider_id = ?')
    .bind(identity.provider, identity.providerId)
    .first<User>()
  if (linked) return linked

  let user = await db.prepare('select * from user where email = ?').bind(identity.email).first<User>()

  if (!user) {
    const id = `u_${random(12)}`
    await db.prepare('insert into user (id, email, name, created_at) values (?, ?, ?, ?)').bind(id, identity.email, identity.name ?? null, Date.now()).run()
    user = (await db.prepare('select * from user where id = ?').bind(id).first<User>())!
  }

  await db.prepare('insert into account (provider, provider_id, user_id) values (?, ?, ?)').bind(identity.provider, identity.providerId, user.id).run()
  return user
}

export const handleTaken = async (db: D1Database, handle: string, except?: string) =>
  Boolean(await db.prepare('select 1 from user where handle = ? and id != ?').bind(handle, except ?? '').first())

export async function updateProfile(db: D1Database, id: string, profile: { handle: string; name: string; avatar: Avatar; bio?: string }) {
  await db
    .prepare('update user set handle = ?, name = ?, avatar_icon = ?, avatar_palette = ?, bio = coalesce(?, bio) where id = ?')
    .bind(profile.handle, profile.name, profile.avatar.icon, profile.avatar.palette, profile.bio ?? null, id)
    .run()

  return (await db.prepare('select * from user where id = ?').bind(id).first<User>())!
}

export const publicUser = ({ id, email, name, handle, avatar_icon, avatar_palette, bio }: User): Me => ({
  id,
  email,
  name,
  handle,
  bio,
  avatar: avatar_icon && avatar_palette ? { icon: avatar_icon, palette: avatar_palette as Palette } : null
})

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
