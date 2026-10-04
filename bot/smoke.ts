import { ASK_DOMAIN, SITE } from './names'

// One real question after a deploy, read for its shape and never its wording, since a model never answers twice the same.
const asked = 'how do I run a script with the CLI?'

const WAIT_MS = 120_000
const LIVE_MS = 60_000
const RETRY_MS = 5_000

const rest = (ms: number) => new Promise((wake) => setTimeout(wake, ms))

// A hostname born in this deploy still waits on its certificate, so a failure is retried until the deadline.
async function reached(until: number): Promise<Response> {
  const response = await fetch(`https://${ASK_DOMAIN}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ question: asked })
  }).catch(() => null)

  if (response?.ok) return response
  if (Date.now() > until) throw new Error(response ? `${ASK_DOMAIN} answered ${response.status}` : `${ASK_DOMAIN} is out of reach`)

  await rest(RETRY_MS)
  return reached(until)
}

// The question armed the Discord loop if the deploy left it unarmed, so a tick has to land within a few of them.
async function heard(until: number): Promise<{ discord?: boolean; ticked?: number | null }> {
  const read = (await (await fetch(`https://${ASK_DOMAIN}/health`)).json()) as { discord?: boolean; ticked?: number | null }
  if (read.discord === false || (read.ticked && Date.now() - read.ticked < 30_000)) return read
  if (Date.now() > until) throw new Error(`the Discord loop has not ticked since ${read.ticked ? new Date(read.ticked).toISOString() : 'it was deployed'}`)

  await rest(RETRY_MS)
  return heard(until)
}

// The reference, the search and a read each fail quietly inside an answer, so each is checked on its own.
const reference = await fetch(`${SITE}/SKILL.md`)
if (!reference.ok || !(await reference.text()).trim()) throw new Error(`${SITE}/SKILL.md answered ${reference.status} with nothing to answer from`)

const shelf = await fetch(`${SITE}/api/search?q=cli`)
if (!shelf.ok) throw new Error(`${SITE}/api/search answered ${shelf.status}`)

const { docs } = (await shelf.json()) as { docs: { href: string }[] }
if (!docs.length) throw new Error(`${SITE}/api/search found nothing for a word its own reference uses`)

const page = `${SITE}/api${docs[0]!.href.split('#')[0]}`
const opened = await fetch(page)
if (!opened.ok || !((await opened.json()) as { body?: string }).body) throw new Error(`${page} answered no page to read`)

const answer = (await (await reached(Date.now() + WAIT_MS)).json()) as { text?: string; sources?: string[]; session?: string }
if (!answer.text) throw new Error('answered with no text')
if (!answer.session) throw new Error('answered without a session, so a follow-up would start over')

console.log(`ok, ${answer.text.length} characters citing ${answer.sources?.length ?? 0} page(s)`)

const loop = await heard(Date.now() + LIVE_MS)
console.log(loop.discord === false ? 'ok, Discord is off' : `ok, the Discord loop ticked ${Math.round((Date.now() - loop.ticked!) / 1000)}s ago`)
