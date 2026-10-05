// How long a bar lingers after a scroll, its shortest length and its gap at each end.
const LINGER = 400
const SHORTEST = 24
const EDGE = 2

/* Where a scroll sits, what it shows of what it holds, and the length the bar runs along. */
export type Span = { offset: number; view: number; total: number; track: number }

/* Draws `bar` along one axis, `edge` short of each end, and drags the scroll through `seek`. */
export function scrollbar(bar: HTMLElement, axis: 'x' | 'y', seek: (offset: number) => void, edge = EDGE) {
  let timer: ReturnType<typeof setTimeout> | undefined
  let frame = 0
  let span: Span
  let length = 0
  // What keeps the bar in view, a pointer on what scrolls, a pointer on the bar or a drag.
  let holding = false
  let over = false
  let dragging = false
  const kept = () => holding || over || dragging

  const place = () => {
    frame = 0
    const room = span.track - 2 * edge
    length = Math.max(SHORTEST, (room * span.view) / span.total)
    const travel = span.total - span.view
    const at = edge + (travel > 0 ? (span.offset / travel) * (room - length) : 0)
    bar.style[axis === 'x' ? 'width' : 'height'] = `${length}px`
    bar.style.transform = axis === 'x' ? `translateX(${at}px)` : `translateY(${at}px)`
  }

  const fade = () => bar.removeAttribute('data-shown')
  const settle = () => {
    clearTimeout(timer)
    if (!kept()) timer = setTimeout(fade, LINGER)
  }

  // Nothing to show when everything already fits.
  const show = (next: Span) => {
    clearTimeout(timer)
    if (next.total <= next.view) return false
    span = next
    frame ||= requestAnimationFrame(place)
    bar.setAttribute('data-shown', '')
    return true
  }

  bar.addEventListener('pointerenter', () => { over = true; clearTimeout(timer) })
  bar.addEventListener('pointerleave', () => { over = false; settle() })

  // The pointer is held by the bar, so a drag keeps going past its edges and outside the window.
  bar.addEventListener('pointerdown', (event) => {
    if (event.button !== 0 || !span) return
    event.preventDefault()
    bar.setPointerCapture(event.pointerId)
    dragging = true
    bar.setAttribute('data-dragging', '')
    const from = axis === 'x' ? event.clientX : event.clientY
    const start = span.offset
    const rate = (span.total - span.view) / Math.max(1, span.track - 2 * edge - length)

    const move = (moved: PointerEvent) => seek(start + ((axis === 'x' ? moved.clientX : moved.clientY) - from) * rate)
    const end = () => {
      dragging = false
      bar.removeAttribute('data-dragging')
      bar.removeEventListener('pointermove', move)
      settle()
    }
    bar.addEventListener('pointermove', move)
    bar.addEventListener('lostpointercapture', end, { once: true })
  })

  return {
    moved(next: Span) {
      if (show(next)) settle()
    },
    // A pointer resting on what scrolls keeps the bar in view until it leaves.
    held(next: Span | null) {
      holding = next !== null
      if (next) show(next)
      else if (!kept()) fade()
    }
  }
}

/* Wires each `[data-bar]` under `root` to the `[data-scroller]` in its box, `edge` short of each end. */
export function thumbs(root: HTMLElement, edge = EDGE) {
  root.querySelectorAll<HTMLElement>('[data-bar]').forEach((bar) => {
    const box = bar.parentElement!
    const scroller = box.querySelector<HTMLElement>('[data-scroller]')!
    const x = bar.dataset.bar === 'x'
    const span = (): Span => x
      ? { offset: scroller.scrollLeft, view: scroller.clientWidth, total: scroller.scrollWidth, track: box.clientWidth }
      : { offset: scroller.scrollTop, view: scroller.clientHeight, total: scroller.scrollHeight, track: box.clientHeight }
    const { moved, held } = scrollbar(bar, x ? 'x' : 'y', (to) => scroller.scrollTo(x ? { left: to } : { top: to }), edge)
    box.addEventListener('pointerenter', (event) => { if (event.pointerType === 'mouse') held(span()) })
    box.addEventListener('pointerleave', (event) => { if (event.pointerType === 'mouse') held(null) })
    scroller.addEventListener('scroll', () => moved(span()), { passive: true })
  })
}
