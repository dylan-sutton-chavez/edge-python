import { DurableObject } from 'cloudflare:workers'
import { stale } from './alarm'
import { answer } from './answer'
import { left, spend } from './budget'
import { PER_TICK, TICK_MS } from './config'
import { calls, channels, reply, since, spoken, whoami, type Message } from './discord'
import { link, remember, rooted, sweep, threaded, turns } from './session'

export type Env = { AI: Ai; DB: D1Database; DISCORD_TOKEN: string; GUILD: string; SITE: string }

// Woken by its own alarm and nudged by the cron, and everything it keeps is in the database so a restart only reads the cursor again.
export class Bot extends DurableObject<Env> {
  async alive() {
    const at = await this.ctx.storage.getAlarm()
    if (!stale(at, Date.now())) return

    console.log('arming', at === null ? 'none' : new Date(at).toISOString())
    await this.ctx.storage.setAlarm(Date.now() + TICK_MS)
  }

  // When the loop last ran and when it runs next, which the deploy reads to know the server is being heard.
  async health() {
    return { alarm: await this.ctx.storage.getAlarm(), ticked: (await this.ctx.storage.get<number>('ticked')) ?? null }
  }

  async alarm() {
    await this.ctx.storage.setAlarm(Date.now() + TICK_MS)
    await this.ctx.storage.put('ticked', Date.now())

    try {
      await this.tick()
    } catch (failure) {
      console.error('tick', failure)
    }
  }

  private async tick() {
    await sweep(this.env)

    const { room, quiet } = await left(this.env)
    if (quiet || room <= 0) return

    const token = this.env.DISCORD_TOKEN
    const self = await whoami(token, this.env.GUILD)
    let budget = Math.min(room, PER_TICK)

    for (const channel of await channels(token, this.env.GUILD)) {
      const after = await this.env.DB.prepare('select last_id from cursor where channel = ?').bind(channel.id).first<string>('last_id')

      // A channel the bot was not let into answers 403, and it reads as quiet rather than stopping the ones after it.
      const fresh = await since(token, channel.id, after).catch(() => [])
      if (!fresh.length) continue

      // Past everything read, the bot's own replies included, so a channel is never walked twice.
      await this.env.DB.prepare('insert or replace into cursor (channel, last_id) values (?, ?)').bind(channel.id, fresh.at(-1)!.id).run()

      // A first pass only learns where the channel is, so nothing said before the bot arrived is answered.
      if (!after) continue

      // Without the MESSAGE CONTENT intent a mention of the bot's role arrives with no text, and a question nobody can read is left alone.
      const asked = fresh.filter((each) => !each.author.bot && each.content.trim() && calls(each, self))

      for (const message of asked.slice(0, budget)) {
        // Each on its own, since the cursor has passed them all and one failure must not lose the rest.
        try {
          await this.respond(channel.id, message)
        } catch (failure) {
          console.error('answering', message.id, failure)
        }

        budget -= 1
      }

      if (budget <= 0) return
    }
  }

  // A reply carries on the thread it answers, and anything else opens one rooted at itself.
  private async respond(channel: string, message: Message) {
    const parent = message.referenced_message?.id
    const id = (parent && (await threaded(this.env, parent))) || (await rooted(message.id))
    const held = await turns(this.env, id, 'discord')

    await spend(this.env)

    const found = await answer(this.env, held, message.content)
    const sent = await reply(this.env.DISCORD_TOKEN, channel, message.id, spoken(found))

    await remember(this.env, id, 'discord', [...held, { role: 'user', content: message.content }, { role: 'assistant', content: found.text }])
    await link(this.env, sent.id, id)
  }
}
