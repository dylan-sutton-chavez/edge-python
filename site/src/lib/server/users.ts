import { random } from '../crypto'
import type { Avatar, Palette } from '../account/avatar'
import type { Me, Public } from '../account/auth'

export type User = {
  id: string
  name: string | null
  handle: string | null
  avatar_icon: number | null
  avatar_palette: string | null
  bio: string | null
  created_at: number
  updated_at: number
  handle_changed_at: number | null
}

// The signed-in row carries the address its codes go to, which a join brings in from account.
export type Holder = User & { email: string | null }

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

/* The user behind an identity, found by its own credential or by an address already on file, and created with both when neither exists. */
export async function upsertUser(db: D1Database, identity: Identity): Promise<User> {
  const own = await byCredential(db, identity.provider, identity.providerId)
  if (own) return own

  const known = await byCredential(db, 'email', identity.email)

  if (known) {
    await linkAccount(db, known.id, identity)
    return known
  }

  const id = `u_${random(12)}`
  const now = Date.now()

  // The address lands beside the provider, so a code to it reaches this account from the first day.
  await db.batch([
    db.prepare('insert into user (id, name, created_at, updated_at) values (?, ?, ?, ?)').bind(id, identity.name ?? null, now, now),
    db.prepare('insert or ignore into account (provider, provider_id, user_id, created_at) values (?, ?, ?, ?)').bind('email', identity.email, id, now),
    db.prepare('insert or ignore into account (provider, provider_id, user_id, created_at) values (?, ?, ?, ?)').bind(identity.provider, identity.providerId, id, now)
  ])

  return (await userById(db, id))!
}

export const handleTaken = async (db: D1Database, handle: string, except?: string) =>
  Boolean(await db.prepare('select 1 from user where handle = ? and id != ?').bind(handle, except ?? '').first())

/* Saves the profile, and starts the wait only when the handle actually moved. */
export async function updateProfile(db: D1Database, id: string, profile: { handle: string; name: string; avatar: Avatar; bio?: string }, moved: boolean) {
  const now = Date.now()

  await db
    .prepare('update user set handle = ?, name = ?, avatar_icon = ?, avatar_palette = ?, bio = coalesce(?, bio), updated_at = ?, handle_changed_at = coalesce(?, handle_changed_at) where id = ?')
    .bind(profile.handle, profile.name, profile.avatar.icon, profile.avatar.palette, profile.bio ?? null, now, moved ? now : null, id)
    .run()

  return (await userById(db, id))!
}

export const publicUser = ({ id, name, handle, avatar_icon, avatar_palette, bio }: User): Public => ({
  id,
  name,
  handle,
  bio,
  avatar: avatar_icon && avatar_palette ? { icon: avatar_icon, palette: avatar_palette as Palette } : null
})

/* The signed-in view, which is the public one plus the address the codes go to. */
export const me = (holder: Holder): Me => ({ ...publicUser(holder), email: holder.email })

export const userById = (db: D1Database, id: string) => db.prepare('select * from user where id = ?').bind(id).first<User>()

export const userByHandle = (db: D1Database, handle: string) => db.prepare('select * from user where handle = ?').bind(handle).first<User>()

/* The address a user's codes go to, which lives in account like any other credential. */
export const addressOf = async (db: D1Database, userId: string) =>
  (await db.prepare("select provider_id from account where user_id = ? and provider = 'email'").bind(userId).first<{ provider_id: string }>())?.provider_id ?? null

export const addressTaken = async (db: D1Database, email: string) =>
  Boolean(await db.prepare("select 1 from account where provider = 'email' and provider_id = ?").bind(email).first())

/* Moves where the codes go, one write because the address lives in one place. False when another account holds it. */
export async function moveAddress(db: D1Database, userId: string, email: string) {
  try {
    const done = await db
      .prepare("update account set provider_id = ? where user_id = ? and provider = 'email'")
      .bind(email, userId)
      .run()

    return done.meta.changes > 0
  } catch {
    // The primary key refuses an address another account already holds, whoever got there first.
    return false
  }
}

export async function linkedProviders(db: D1Database, userId: string) {
  const { results } = await db.prepare('select provider from account where user_id = ?').bind(userId).all<{ provider: string }>()
  return results.map((row) => row.provider)
}

// Links an OAuth identity to the signed-in user, one already linked elsewhere stays where it is.
export async function linkAccount(db: D1Database, userId: string, identity: Identity) {
  await db
    .prepare('insert or ignore into account (provider, provider_id, user_id, created_at) values (?, ?, ?, ?)')
    .bind(identity.provider, identity.providerId, userId, Date.now())
    .run()
}

export async function unlinkAccount(db: D1Database, userId: string, provider: string) {
  await db.prepare('delete from account where user_id = ? and provider = ?').bind(userId, provider).run()
}

// Sessions, credentials and tokens all cascade from the user row, so one delete is the whole account.
export const deleteUser = (db: D1Database, id: string) => db.prepare('delete from user where id = ?').bind(id).run()
