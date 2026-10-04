import { answer } from './answer'
import { Bot, type Env } from './bot'
import { left, spend } from './budget'
import { digest, random, remember, turns } from './session'

export { Bot }

type Worker = Env & { BOT: DurableObjectNamespace<Bot>; DISCORD: string; ASK_IP: RateLimit }

const json = (data: unknown, status = 200) => Response.json(data, { status })

// One name, so every wake reaches the same object and no channel is read twice.
const bot = (env: Worker) => env.BOT.get(env.BOT.idFromName('discord'))

export default {
  async fetch(request, env, ctx) {
    // Any request arms a loop a deploy left unarmed, rather than the cron five minutes later.
    if (env.DISCORD === '1') ctx.waitUntil(bot(env).alive())

    // Read by the deploy, since a loop that died answers nothing and says nothing.
    if (request.method === 'GET' && new URL(request.url).pathname === '/health') {
      return json(env.DISCORD === '1' ? await bot(env).health() : { discord: false })
    }

    if (request.method !== 'POST') return json({ error: 'Send a question with POST.' }, 405)

    const sent = (await request.json().catch(() => ({}))) as { question?: unknown; session?: unknown }
    const asked = typeof sent.question === 'string' ? sent.question.trim() : ''
    if (!asked) return json({ error: 'Send a question.' }, 400)

    // Per address, since the day's budget is shared with the server and one caller could spend it all.
    const from = request.headers.get('cf-connecting-ip')
    if (from && !(await env.ASK_IP.limit({ key: from })).success) return json({ error: 'Too many questions. Try again in a minute.' }, 429)

    const { room, quiet } = await left(env)
    if (quiet) return json({ error: 'Not answering for now.' }, 503)
    if (room <= 0) return json({ error: 'Answering nothing more today. Try again tomorrow.' }, 429)

    // The caller keeps the secret and only its digest is stored, and one that went quiet opens a new conversation.
    const secret = typeof sent.session === 'string' && sent.session.length <= 64 ? sent.session : random()
    const id = await digest(secret)
    const held = await turns(env, id, 'http')

    await spend(env)

    const found = await answer(env, held, asked).catch((failure) => console.error('answering', failure))
    if (!found) return json({ error: 'No answer came back. Try again.' }, 502)

    await remember(env, id, 'http', [...held, { role: 'user', content: asked }, { role: 'assistant', content: found.text }])
    return json({ ...found, session: secret })
  },

  // The alarm drives every tick, and this only re-arms one that strayed.
  async scheduled(_event, env) {
    if (env.DISCORD === '1') await bot(env).alive()
  }
} satisfies ExportedHandler<Worker>
