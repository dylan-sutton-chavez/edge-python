import { HISTORY_MS, MAX_QUESTION, MAX_SAID, SESSION_MS, TURNS } from './config'

export type Turn = { role: 'user' | 'assistant'; content: string }

export type Kind = 'discord' | 'http'

type Env = { DB: D1Database }

const b64url = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes))
    .replaceAll('+', '-')
    .replaceAll('/', '_')
    .replace(/=+$/, '')

export const random = (bytes = 32) => b64url(crypto.getRandomValues(new Uint8Array(bytes)))

export const digest = async (text: string) => b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))))

// Keyed by the message that opened a thread rather than by who wrote it, so one person holds as many as they start.
export const rooted = (message: string) => digest(`discord:${message}`)

// Empty for a conversation that never existed or went quiet, and the kind keeps an id from one door out of the other.
export async function turns(env: Env, id: string, kind: Kind): Promise<Turn[]> {
  const row = await env.DB
    .prepare('select turns from session where id = ? and kind = ? and last_at > ? and born_at > ?')
    .bind(id, kind, Date.now() - SESSION_MS, Date.now() - HISTORY_MS)
    .first<{ turns: string }>()

  return row ? (JSON.parse(row.turns) as Turn[]) : []
}

// Only the tail is kept, and an answer is kept shorter than a question since a model copies the length of its last one.
const capped = (held: Turn[]) =>
  held.slice(-TURNS).map((each) => ({ role: each.role, content: each.content.slice(0, each.role === 'assistant' ? MAX_SAID : MAX_QUESTION) }))

export async function remember(env: Env, id: string, kind: Kind, held: Turn[]) {
  const now = Date.now()

  await env.DB
    .prepare('insert into session (id, kind, turns, born_at, last_at) values (?, ?, ?, ?, ?) on conflict(id) do update set turns = excluded.turns, last_at = excluded.last_at')
    .bind(id, kind, JSON.stringify(capped(held)), now, now)
    .run()
}

// What a reply points at, so its thread is one lookup rather than a walk back through the chain.
export const link = (env: Env, message: string, id: string) =>
  env.DB.prepare('insert or replace into thread (message_id, session) values (?, ?)').bind(message, id).run()

export const threaded = (env: Env, message: string) => env.DB.prepare('select session from thread where message_id = ?').bind(message).first<string>('session')

// Run on every tick, since nothing else would delete a conversation that simply stopped.
export async function sweep(env: Env) {
  const now = Date.now()

  await env.DB.batch([
    env.DB.prepare('delete from thread where session in (select id from session where last_at < ? or born_at < ?)').bind(now - SESSION_MS, now - HISTORY_MS),
    env.DB.prepare('delete from session where last_at < ? or born_at < ?').bind(now - SESSION_MS, now - HISTORY_MS)
  ])
}
