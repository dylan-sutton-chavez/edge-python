// Whether the reader asked for a page that holds still.
export const still = matchMedia('(prefers-reduced-motion: reduce)')

// A text blurred out and back in, the way a note or a value changes in place.
export const clear = { opacity: 1, filter: 'blur(0)' }
export const blurred = { opacity: 0, filter: 'blur(4px)' }
export const ease = 'cubic-bezier(0.2, 0.8, 0.2, 1)'

export const wait = (ms: number) => new Promise((done) => setTimeout(done, ms))

/* Runs `start` once, the first time most of `element` comes into view. */
export function seen(element: Element, start: () => void) {
  const watch = new IntersectionObserver(([entry]) => {
    if (!entry?.isIntersecting) return
    watch.disconnect()
    start()
  }, { threshold: 0.4 })
  watch.observe(element)
}

/* Blurs `element` out, gives it `text` and `changed` a turn to follow it, then blurs it back in. */
export async function swap(element: HTMLElement, text: string, changed?: () => void) {
  await element.animate([clear, blurred], { duration: 220, easing: 'ease-in', fill: 'forwards' }).finished
  element.textContent = text
  changed?.()
  element.animate([blurred, clear], { duration: 380, easing: ease, fill: 'forwards' })
}
