// Collapses a runnable fence and the ```output right after it into one embedded editor.
const RUNNABLE = 'edge-python'
const OUTPUT = 'output'

type Node = { type: string; lang?: string | null; value?: string }
type Root = { children: Node[] }
type File = { data: { astro?: { frontmatter?: Record<string, unknown> } } }

export type Pair = { code: string; output: string }

const embed = (pair: Pair) => ({
  type: 'mdxJsxFlowElement',
  name: 'Playground',
  attributes: [
    { type: 'mdxJsxAttribute', name: 'code', value: pair.code },
    { type: 'mdxJsxAttribute', name: 'output', value: pair.output }
  ],
  children: []
})

// Where a pair sat, for a renderer that gets HTML back as a string and places the editor itself.
export const MARK = /<!--playground:(\d+)-->/
const mark = (at: number) => ({ type: 'html', value: `<!--playground:${at}-->` })

// The frontmatter key the collected pairs come back under, since that is how a plugin answers a render.
export const PAIRS = 'playgrounds'

/* Collecting, the pairs come back through the frontmatter and a marker holds each place, which is how a page rendered at request time reaches the same component. Otherwise the pair becomes the MDX element the build compiles. */
export function remarkPlayground(collect = false) {
  return (tree: Root, file: File) => {
    const found: Pair[] = []

    for (let i = tree.children.length - 1; i >= 0; i--) {
      const node = tree.children[i]
      if (node?.type !== 'code' || node.lang !== RUNNABLE) continue

      const next = tree.children[i + 1]
      const paired = next?.type === 'code' && next.lang === OUTPUT
      const pair = { code: node.value ?? '', output: paired ? (next.value ?? '') : '' }

      tree.children.splice(i, paired ? 2 : 1, collect ? mark(found.push(pair) - 1) : embed(pair))
    }

    if (!collect) return

    const astro = (file.data.astro ??= {})
    astro.frontmatter = { ...astro.frontmatter, [PAIRS]: found }
  }
}
