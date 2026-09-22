import { defineMiddleware } from 'astro:middleware'
import { env } from 'cloudflare:workers'
import { readSession } from './lib/server/session'
import { me } from './lib/server/users'

export const onRequest = defineMiddleware(async ({ cookies, locals }, next) => {
  const user = await readSession(env.DB, cookies)
  locals.user = user && me(user)
  return next()
})
