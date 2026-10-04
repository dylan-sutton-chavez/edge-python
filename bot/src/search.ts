import { MAX_PASSAGE, PASSAGES } from './config'

export type Found = { title: string; where: string; href: string; snippet: string }

export type Page = Found & { text: string }

// The index brackets what it matched with control characters, which mean nothing to a model.
const MARKS = /[\u0001\u0002]/g

const FENCE = /^\s*```/
const HEADING = /^#{2,3}\s+(.+?)\s*$/

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

export async function search(site: string, query: string): Promise<Page[]> {
  const response = await fetch(`${site}/api/search?q=${encodeURIComponent(query)}`)
  if (!response.ok) throw new Error(`search answered ${response.status}`)

  // One hit from each shelf in turn, so the docs that matched never crowd out a package.
  const shelves = Object.values((await response.json()) as Record<string, Found[]>)
  const hits = Array.from({ length: PASSAGES }, (_, at) => shelves.flatMap((shelf) => shelf[at] ?? [])).flat().slice(0, PASSAGES)

  return Promise.all(hits.map(async (hit) => ({ ...hit, text: await read(site, hit) })))
}
