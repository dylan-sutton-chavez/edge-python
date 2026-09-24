import { ORDER } from './convention'

export type Doc = { id: string; slug: string; title: string }
export type Section = { label: string | null; docs: Doc[] }

type Entry = { id: string; data: { title?: string } }

// A collection entry arrives without its extension and a packed page with one, and a slug carries neither.
const name = (segment: string) => segment.replace(ORDER, '').replace(/\.mdx$/, '')

/* The url a stored path answers under, which is the path with what only the store needed taken off. */
export const slugOf = (path: string) => path.split('/').map(name).join('/')

function label(segment: string) {
  const words = name(segment).replaceAll('-', ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

export function tree(entries: Entry[], base: string): Section[] {
  const prefix = base.replace(/^\.\/?/, '').replace(/\/$/, '')
  const sections: Section[] = []
  const found = new Map<string, Section>()

  const place = (key: string, heading: string | null, doc: Doc) => {
    let section = found.get(key)

    if (!section) {
      section = { label: heading, docs: [] }
      found.set(key, section)
      sections.push(section)
    }

    section.docs.push(doc)
  }

  const inside = entries
    .filter((entry) => !prefix || entry.id.startsWith(`${prefix}/`))
    .map((entry) => ({ entry, segments: (prefix ? entry.id.slice(prefix.length + 1) : entry.id).split('/') }))
    .sort((a, b) => a.entry.id.localeCompare(b.entry.id, undefined, { numeric: true }))

  for (const { entry, segments } of inside) {
    if (segments.length > 2) throw new Error(`Docs nest one folder deep at most, "${entry.id}" goes deeper.`)

    const folder = segments.length > 1 ? segments[0]! : null

    place(folder ?? '', folder && label(folder), {
      id: entry.id,
      slug: segments.map(name).join('/'),
      title: entry.data.title ?? label(segments.at(-1)!)
    })
  }

  return sections
}
