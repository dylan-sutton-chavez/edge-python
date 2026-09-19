import { defineMiddleware } from 'astro:middleware'
import { env } from 'cloudflare:workers'
import { readSession } from './lib/server/session'
import { publicUser } from './lib/server/users'

export const onRequest = defineMiddleware(async ({ cookies, locals }, next) => {
  const user = await readSession(env.DB, cookies)
  locals.user = user && publicUser(user)
  return next()
})
