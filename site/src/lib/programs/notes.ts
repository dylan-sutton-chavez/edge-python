import { ease, still, swap } from './motion'

/* Swaps the note of `pill` for the next every few seconds, never under the pointer. */
export function notes(pill: HTMLElement) {
  const note = pill.firstElementChild as HTMLElement
  const list: string[] = JSON.parse(pill.dataset.notes!)
  let at = 0
  if (list.length < 2 || still.matches) return

  setInterval(() => {
    if (document.hidden || pill.matches(':hover')) return
    at = (at + 1) % list.length
    const from = pill.offsetWidth
    swap(note, list[at]!, () => pill.animate([{ width: `${from}px` }, { width: `${pill.offsetWidth}px` }], { duration: 380, easing: ease }))
  }, 3500)
}
