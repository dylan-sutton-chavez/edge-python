import type { Answer } from './answer'
import { DISCORD_API, FETCH_LIMIT, MAX_MESSAGE } from './config'
import { prose } from './prose'

export type Message = {
  id: string
  content: string
  author: { id: string; bot?: boolean }
  mentions: { id: string }[]
  mention_roles: string[]
  referenced_message?: { id: string; author: { id: string } }
}

export type Self = { id: string; role?: string }

async function call<T>(token: string, path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(`${DISCORD_API}${path}`, {
    ...init,
    headers: { authorization: `Bot ${token}`, 'content-type': 'application/json', ...init.headers }
  })

  if (!response.ok) throw new Error(`discord answered ${response.status} to ${path}`)
  return response.json() as Promise<T>
}

// The bot and the role Discord made for it on joining, since a person mentioning it picks either one from the same list.
export async function whoami(token: string, guild: string): Promise<Self> {
  const { id } = await call<{ id: string }>(token, '/users/@me')
  const roles = await call<{ id: string; tags?: { bot_id?: string } }[]>(token, `/guilds/${guild}/roles`)

  return { id, role: roles.find((each) => each.tags?.bot_id === id)?.id }
}

export const channels = async (token: string, guild: string) =>
  (await call<{ id: string; type: number }[]>(token, `/guilds/${guild}/channels`)).filter((each) => each.type === 0)

// Oldest first, because a cursor only moves forward.
export const since = async (token: string, channel: string, after: string | null) =>
  (await call<Message[]>(token, `/channels/${channel}/messages${after ? `?after=${after}&limit=${FETCH_LIMIT}` : '?limit=1'}`)).reverse()

export const calls = (message: Message, self: Self) =>
  message.mentions.some((each) => each.id === self.id) ||
  (self.role !== undefined && message.mention_roles.includes(self.role)) ||
  message.referenced_message?.author.id === self.id

export const reply = (token: string, channel: string, to: string, content: string) =>
  call<Message>(token, `/channels/${channel}/messages`, {
    method: 'POST',
    // Nobody is pinged by an answer, since a mention the bot chose would be noise.
    body: JSON.stringify({ content, message_reference: { message_id: to }, allowed_mentions: { parse: [] } })
  })

const FENCE = /^\s*```\s*$/
const RULE = /^\s*([-*_])\1{2,}\s*$/
const LIST = /^\s*([-*+>]|\d+\.)\s/

// A sentence ends where a full stop meets the capital that opens the next, so `edge.json` and `e.g. the` are never taken for one.
const BREAK = /(?<=[.!?])\s+(?=[A-ZÁÉÍÓÚÑ¿¡`"(0-9])/g
const LONG = 360
const HALF = 120

// Discord leaves a masked link raw when its label is an address, which is how it stops one hiding behind another.
const HIDDEN = /\[https?:\/\/[^\]\s]+\]\((https?:\/\/[^)\s]+)\)/g

// A paragraph too long for a chat is cut at the sentence end nearest its middle, again while still too long, and no half is left shorter than a line.
function paragraphs(line: string): string[] {
  if (line.length <= LONG || LIST.test(line)) return [line]

  const cuts = [...line.matchAll(BREAK)].map((each) => each.index).filter((at) => at >= HALF && line.length - at >= HALF)
  if (!cuts.length) return [line]

  const middle = line.length / 2
  const at = cuts.reduce((best, each) => (Math.abs(each - middle) < Math.abs(best - middle) ? each : best))

  return [...paragraphs(line.slice(0, at)), ...paragraphs(line.slice(at).trim())]
}

// Discord breaks a line after a block by itself, so a blank line goes before a block and never after one, and never two anywhere.
function tidy(text: string) {
  const out: string[] = []
  let inside = false

  const blank = () => {
    if (out.length && out.at(-1) !== '' && out.at(-1) !== '```') out.push('')
  }

  for (const raw of text.split('\n')) {
    if (inside) {
      inside = !FENCE.test(raw)
      out.push(inside ? raw : '```')
      continue
    }

    const line = raw.trimEnd()

    if (line.startsWith('```')) {
      blank()
      // One word, since the chat shows a second as code.
      out.push(line.replace(/^(```\S*).*$/, '$1'))
      inside = true
    } else if (!line || RULE.test(line)) blank()
    else paragraphs(line).forEach((part, at) => out.push(...(at ? ['', part] : [part])))
  }

  // A block a cut left open is closed rather than left swallowing the rest.
  if (inside) out.push('```')
  return out.join('\n').trim()
}

// The cited number becomes a link with its parentheses, which renders because a number is nothing like an address.
const linked = (text: string, sources: string[]) =>
  prose(text, (part) =>
    part.replace(HIDDEN, '<$1>').replace(/\[(\d+)\]/g, (whole, mark: string) => {
      const url = sources[Number(mark) - 1]
      return url ? `[(${mark})](<${url}>)` : whole
    })
  )

// Discord refuses a longer message outright, so it is cut at a line break where there is one.
function fits(content: string) {
  if (content.length <= MAX_MESSAGE) return content

  const held = content.slice(0, MAX_MESSAGE - 1)
  const line = held.lastIndexOf('\n')

  return `${line > MAX_MESSAGE / 2 ? held.slice(0, line) : held}…`
}

export const spoken = (answer: Answer) => fits(linked(tidy(answer.text), answer.sources))
