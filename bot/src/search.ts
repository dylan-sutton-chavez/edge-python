import { MAX_PASSAGE, PASSAGES } from './config'

export type Found = { title: string; where: string; href: string; snippet: string }

export type Page = Found & { text: string }

// The index brackets what it matched with control characters, which mean nothing to a model.
const MARKS = /[\u0001\u0002]/g

const FENCE = /^\s*```/
const HEADING = /^#{2,3}\s+(.+?)\s*$/

// The first item of every list, then the second, so no list crowds out the others.
const interleaved = <T>(lists: T[][]) =>
  Array.from({ length: Math.max(0, ...lists.map((list) => list.length)) }, (_, at) => lists.flatMap((list) => list[at] ?? [])).flat()

// Cut at second and third level headings outside code the way the site's search cuts a page, so a hit's title names its part.
export function section(body: string, heading?: string) {
  const parts: { heading?: string; lines: string[] }[] = [{ lines: [] }]
  let open = false

  for (const line of body.split('\n')) {
    if (FENCE.test(line)) open = !open

    const found = !open && HEADING.exec(line)
    if (found) parts.push({ heading: found[1], lines: [line] })
    else parts.at(-1)!.lines.push(line)
  }

  return (parts.find((part) => part.heading === heading) ?? parts[0]!).lines.join('\n').trim()
}

// What a hit points at, read as data under /api, and its snippet when that read fails.
async function read(site: string, hit: Found) {
  const [path, anchor] = hit.href.split('#')
  const data = (await fetch(`${site}/api${path}`)
    .then((response) => (response.ok ? response.json() : null))
    .catch(() => null)) as { body?: string } | null

  if (!data) return hit.snippet.replace(MARKS, '')
  return (data.body === undefined ? JSON.stringify(data) : section(data.body, anchor && hit.title)).slice(0, MAX_PASSAGE)
}

async function shelves(site: string, query: string): Promise<Found[]> {
  const response = await fetch(`${site}/api/search?q=${encodeURIComponent(query)}`)
  if (!response.ok) throw new Error(`search answered ${response.status}`)

  return interleaved(Object.values((await response.json()) as Record<string, Found[]>))
}

// Every query at once, one hit from each in turn and each page once.
export async function find(site: string, queries: string[]) {
  const lists = await Promise.all(queries.map((query) => shelves(site, query).catch((): Found[] => [])))
  return [...new Map(interleaved(lists).map((hit) => [hit.href, hit])).values()]
}

export async function search(site: string, queries: string[]): Promise<Page[]> {
  const hits = (await find(site, queries)).slice(0, PASSAGES)
  return Promise.all(hits.map(async (hit) => ({ ...hit, text: await read(site, hit) })))
}

// The page a term names, a hit titled with it before the first one found.
export async function closest(site: string, term: string) {
  const hits = await find(site, [term])
  return hits.find((hit) => hit.title.replace(/`/g, '').toLowerCase() === term.toLowerCase()) ?? hits[0]
}
