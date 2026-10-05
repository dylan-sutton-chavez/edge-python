import { createHighlight } from '../playground/highlight'
import { seen, still, swap, wait } from './motion'

// How long apart the steps land, how long the proof rests once solved, and how fast it rewinds.
const STEP = 900
const REST = 4500
const REWIND = 160

/* One finding walked in `root`, each step lighting its line until the value that proves it resolves. */
export function proof(root: HTMLElement) {
  const listing = root.querySelector<HTMLElement>('[data-listing]')!
  const numbers = root.querySelectorAll<HTMLElement>('[data-number]')
  const steps = root.querySelectorAll<HTMLElement>('[data-step]')
  const value = root.querySelector<HTMLElement>('[data-witness]')!
  const solved = value.textContent!
  const source = listing.textContent!
  let lit = steps.length - 1

  // Each lit step marks its line, the sink in red, and joins its guide to a lit sibling.
  const mark = () => {
    const rows = listing.querySelectorAll<HTMLElement>('.line')
    steps.forEach((step, at) => {
      const line = Number(step.dataset.line) - 1
      const hit = at <= lit && step.hasAttribute('data-hit')
      step.toggleAttribute('data-more', at > 0 && at < lit)
      for (const each of [rows[line], numbers[line]]) {
        each?.toggleAttribute('data-on', at <= lit && !hit)
        each?.toggleAttribute('data-hit', hit)
      }
    })
  }

  // Swapped in only once the grammar has split it into lines, which the steps light.
  const highlight = createHighlight(() => {
    const html = highlight(source, listing.dataset.lang)
    if (!html.includes('class="line"')) return
    listing.innerHTML = html
    mark()
  })
  mark()

  const resolve = (text: string, done: boolean) => swap(value, text, () => value.toggleAttribute('data-solved', done))
  const show = (step: HTMLElement, on: boolean, ms: number) => step.animate([{ opacity: on ? 0 : 1 }, { opacity: on ? 1 : 0 }], { duration: ms, fill: 'forwards' })

  const walk = async () => {
    lit = -1
    mark()
    steps.forEach((step) => show(step, false, 0))
    await resolve('α', false)
    for (;;) {
      for (const [at, step] of steps.entries()) {
        await wait(STEP)
        lit = at
        mark()
        show(step, true, 200)
      }
      await wait(STEP)
      await resolve(solved, true)
      await wait(REST)
      // Rewound fast, the value back to its symbol and then each step undone from the last.
      await resolve('α', false)
      for (const [at, step] of [...steps.entries()].reverse()) {
        await wait(REWIND)
        lit = at - 1
        mark()
        show(step, false, 120)
      }
      await wait(700)
    }
  }

  // A still page keeps the finding as it renders, solved.
  if (!still.matches) seen(root, walk)
}
