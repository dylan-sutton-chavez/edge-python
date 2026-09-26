import Slugger from 'github-slugger'

/* A stretch of a page under one heading, which is what a reader is sent to rather than the page that holds it. The anchor is slugged the way the renderer slugs its headings, so the link lands where the words are. */
export type Part = { section: string; anchor: string; body: string }

// A fence can hold a line that looks like a heading, so the walk tracks whether it is inside one.
const FENCE = /^\s*```/
const HEADING = /^#{2,3}\s+(.+?)\s*$/

/* Cuts a page at its second- and third-level headings. What comes before the first one belongs to the page itself, which is why that part carries no anchor. */
export function parts(body: string): Part[] {
  const slugger = new Slugger()
  const found: Part[] = [{ section: '', anchor: '', body: '' }]
  let open = false

  for (const line of body.split('\n')) {
    if (FENCE.test(line)) open = !open

    const heading = !open && HEADING.exec(line)

    if (heading) {
      found.push({ section: heading[1]!, anchor: slugger.slug(heading[1]!), body: '' })
      continue
    }

    found[found.length - 1]!.body += `${line}\n`
  }

  return found.filter((part) => part.body.trim().length > 0)
}
