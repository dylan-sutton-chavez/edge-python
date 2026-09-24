import { defineMiddleware } from 'astro:middleware'
import { env } from 'cloudflare:workers'
import { readSession } from './lib/server/session'
import { me } from './lib/server/users'
import { drafted } from './draft'

export const onRequest = defineMiddleware(async ({ cookies, locals, url }, next) => {
  const user = await readSession(env.DB, cookies)
  locals.user = user && me(user)
  return drafted(url, next)
})
