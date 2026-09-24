import { createMarkdownProcessor, type MarkdownRenderer } from '@astrojs/markdown-remark'
import { front } from './convention'
import { MARK, PAIRS, remarkPlayground, type Pair } from './remark-playground'

type Heading = { type: string; depth?: number }

/* Drops the page's own top-level heading, since the package name holds the h1 here and the title is shown above the prose. The convention allows exactly one, so this takes it and leaves every other heading alone. */
const remarkTitle = () => (tree: { children: Heading[] }) => {
  const at = tree.children.findIndex((node) => node.type === 'heading' && node.depth === 1)
  if (at >= 0) tree.children.splice(at, 1)
}

/* A page as the route lays it out, prose already HTML and every runnable fence lifted out, so the markup for an editor lives in the one component instead of in a string. */
export type Block = { kind: 'prose'; html: string } | ({ kind: 'playground' } & Pair)

export type Rendered = { title: string; description: string; blocks: Block[] }

// One processor for the isolate. The pairs ride back on each render's own frontmatter, so two renders cannot cross.
let building: Promise<MarkdownRenderer> | null = null

/* Markdown with no highlighting, because the client paints code once it has the grammar and a page should not wait on one. */
const processor = () =>
  (building ??= createMarkdownProcessor({ syntaxHighlight: false, remarkPlugins: [() => remarkPlayground(true), remarkTitle] }))

/* The pages a bundle carried, in the shape the aside orders them by. A title the walk cannot read leaves the segment to name the page, which is what a tree does for a page without one. */
export const entries = (docs: Record<string, string>) =>
  Object.entries(docs).map(([path, body]) => ({ id: path, data: { title: titled(path, body) } }))

function titled(path: string, body: string) {
  try {
    return front(path, body).keys.get('title')
  } catch {
    return undefined
  }
}

/* A stored page as the package route renders it. The frontmatter is read by the same walk that refused the page at publish, so a title here is a title that was checked for. */
export async function render(page: string, text: string): Promise<Rendered> {
  const { keys, body } = front(page, text)
  const { code, metadata } = await (await processor()).render(body)

  const found = (metadata.frontmatter[PAIRS] ?? []) as Pair[]
  const blocks: Block[] = []

  // A capturing split alternates the prose and the index of the pair that sat between two runs of it.
  code.split(MARK).forEach((piece, at) => {
    if (at % 2 === 0) {
      if (piece.trim()) blocks.push({ kind: 'prose', html: piece })
      return
    }

    const pair = found[Number(piece)]
    if (pair) blocks.push({ kind: 'playground', ...pair })
  })

  return { title: keys.get('title') ?? page, description: keys.get('description') ?? '', blocks }
}
