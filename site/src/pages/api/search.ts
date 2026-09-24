import type { APIRoute } from 'astro'
import { getCollection } from 'astro:content'
import { env } from 'cloudflare:workers'
import { json } from '../../lib/server/http'
import { MARK, named, searched } from '../../lib/server/packages'
import { parts } from '../../lib/docs/sections'
import { slugOf, tree } from '../../lib/docs/tree'

// A trigram needs three characters to be a term, so a shorter query matches names and headings instead.
const TERM = 3
const MAX = 64
const ROOM = 50
const KEEP = 6

export type Found = { title: string; where: string; href: string; snippet: string }

/* One query over two corpora, because a visitor asking about `receive` does not know whether the answer is in the reference or in somebody's package. The site's pages ship inside this worker, so they are scanned here, while a package's pages live in the index the publish route fills. */
export const GET: APIRoute = async ({ url }) => {
  const asked = (url.searchParams.get('q') ?? '').trim().slice(0, MAX)
  if (!asked) return json({ docs: [], packages: [], people: [] })

  const [docs, packages, people] = await Promise.all([ours(asked), theirs(asked), them(asked)])

  return json({ docs, packages, people })
}

/* Whoever publishes, by handle or by the name they chose, because a reader who remembers the author and not the package still knows where to look. */
async function them(asked: string): Promise<Found[]> {
  const { results } = await env.DB
    .prepare(
      `select u.handle, u.name, count(p.name) as held from user u
         left join package p on p.user_id = u.id
       where u.handle is not null and (u.handle like ?1 or u.name like ?1)
       group by u.id
       order by case when u.handle = ?2 then 0 when u.handle like ?3 then 1 else 2 end, held desc
       limit ?4`
    )
    .bind(`%${asked}%`, asked, `${asked}%`, KEEP)
    .all<{ handle: string; name: string | null; held: number }>()

  return results.map((each) => ({
    title: `@${each.handle}`,
    where: each.name ?? 'Person',
    href: `/@${each.handle}`,
    snippet: each.held === 1 ? '1 package' : `${each.held} packages`
  }))
}

/* The site's own sections, read from the same entries the docs route renders, so a result can never name a page that is no longer there. */
async function ours(asked: string): Promise<Found[]> {
  const entries = await getCollection('docs')
  const found: { hit: Found; score: number }[] = []

  for (const section of tree(entries, '')) {
    for (const doc of section.docs) {
      const body = entries.find((entry) => entry.id === doc.id)?.body ?? ''

      for (const part of parts(body)) {
        const where = part.section || section.label || 'Docs'
        const at = part.body.toLowerCase().indexOf(asked.toLowerCase())
        const named = `${doc.title} ${where}`.toLowerCase().indexOf(asked.toLowerCase())

        if (at < 0 && named < 0) continue

        found.push({
          score: ranked(asked, doc.title, where, part.body),
          hit: {
            title: part.section || doc.title,
            where: part.section ? `${section.label ?? 'Docs'} · ${doc.title}` : (section.label ?? 'Docs'),
            href: `/docs/${doc.slug}${part.anchor ? `#${part.anchor}` : ''}`,
            snippet: at < 0 ? trimmed(part.body) : around(part.body, at, asked.length)
          }
        })
      }
    }
  }

  return best(found)
}

/* Published packages, by name first because that is what most people type, then by where the words land inside their documentation. */
async function theirs(asked: string): Promise<Found[]> {
  const byName = await env.DB
    .prepare(
      `select p.name, v.description from package p
         join version v on v.package = p.name
       where p.name like ?1 and v.published_at = (select max(published_at) from version where package = p.name and yanked_at is null)
       order by case when p.name = ?2 then 0 when p.name like ?3 then 1 else 2 end, p.downloads desc
       limit ?4`
    )
    .bind(`%${asked}%`, asked, `${asked}%`, KEEP)
    .all<{ name: string; description: string | null }>()

  const found: Found[] = byName.results.map((each) => ({
    title: each.name,
    where: 'Package',
    href: `/package/${each.name}`,
    snippet: each.description ?? ''
  }))

  if (asked.length < TERM) return found

  const { results } = await searched(env.DB, asked)
  const held = new Set(found.map((each) => each.title))

  for (const hit of results) {
    if (found.length >= KEEP) break
    if (held.has(hit.title) && !hit.section) continue

    found.push({
      title: hit.section || hit.title,
      where: hit.section ? `${hit.package} · ${hit.title}` : hit.package,
      href: `/package/${hit.package}/${slugOf(hit.path)}${hit.anchor ? `#${hit.anchor}` : ''}`,
      snippet: hit.snippet
    })
  }

  return found
}

/* What a reader means by relevant. A heading that names the query beats a title that does, both beat prose, and a match near the top of a section beats one buried in it. */
function ranked(asked: string, title: string, section: string, body: string) {
  const asks = asked.toLowerCase()
  const at = body.toLowerCase().indexOf(asks)

  let score = 0
  if (section.toLowerCase() === asks) score += 200
  if (section.toLowerCase().includes(asks)) score += 80
  if (title.toLowerCase() === asks) score += 120
  if (title.toLowerCase().includes(asks)) score += 40
  if (at >= 0) score += 20 - Math.min(20, Math.floor(at / 200))

  return score
}

const best = (found: { hit: Found; score: number }[]) =>
  found
    .sort((a, b) => b.score - a.score)
    .slice(0, KEEP)
    .map((each) => each.hit)

/* A window of the section around the match, the run bracketed the same way the index brackets its own. */
function around(body: string, at: number, length: number) {
  const from = Math.max(0, at - ROOM)
  const to = Math.min(body.length, at + length + ROOM)

  const lead = from > 0 ? '…' : ''
  const tail = to < body.length ? '…' : ''
  const marked = `${body.slice(from, at)}${MARK.open}${body.slice(at, at + length)}${MARK.close}${body.slice(at + length, to)}`

  return `${lead}${marked}${tail}`.replace(/\s+/g, ' ')
}

const trimmed = (body: string) => `${body.trim().replace(/\s+/g, ' ').slice(0, ROOM * 2)}…`
