import { PER_DAY } from './config'

type Env = { DB: D1Database }

const today = () => new Date().toISOString().slice(0, 10)

// A day opens with the switch the last one closed with, so a silenced bot stays silent until someone sets it back.
export async function left(env: Env) {
  await env.DB.prepare('insert or ignore into budget (day, quiet) values (?, coalesce((select quiet from budget order by day desc limit 1), 0))').bind(today()).run()

  const row = await env.DB.prepare('select spent, quiet from budget where day = ?').bind(today()).first<{ spent: number; quiet: number }>()
  return { room: PER_DAY - (row?.spent ?? 0), quiet: Boolean(row?.quiet) }
}

export const spend = (env: Env, n = 1) => env.DB.prepare('update budget set spent = spent + ? where day = ?').bind(n, today()).run()
