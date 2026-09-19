// Collapses a runnable fence and the ```output right after it into one embedded editor.
const RUNNABLE = 'edge-python'
const OUTPUT = 'output'

type Node = { type: string; lang?: string | null; value?: string }
type Root = { children: Node[] }

const embed = (code: string, output: string) => ({
  type: 'mdxJsxFlowElement',
  name: 'Playground',
  attributes: [
    { type: 'mdxJsxAttribute', name: 'code', value: code },
    { type: 'mdxJsxAttribute', name: 'output', value: output }
  ],
  children: []
})

export function remarkPlayground() {
  return (tree: Root) => {
    for (let i = tree.children.length - 1; i >= 0; i--) {
      const node = tree.children[i]
      if (node?.type !== 'code' || node.lang !== RUNNABLE) continue

      const next = tree.children[i + 1]
      const paired = next?.type === 'code' && next.lang === OUTPUT

      tree.children.splice(i, paired ? 2 : 1, embed(node.value ?? '', paired ? (next.value ?? '') : ''))
    }
  }
}
