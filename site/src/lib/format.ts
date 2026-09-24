const UNITS = ['B', 'KB', 'MB']

/* A size a person reads, one decimal until it is a whole unit, since an artifact is the one number a listing shows about its weight. */
export function bytes(size: number) {
  let at = 0
  let left = size

  while (left >= 1024 && at < UNITS.length - 1) {
    left /= 1024
    at++
  }

  return `${at === 0 ? left : left.toFixed(1)} ${UNITS[at]}`
}

/* A count past a thousand reads shorter than it counts, which is all a download figure has to do. */
export const count = (total: number) => (total < 1000 ? String(total) : `${(total / 1000).toFixed(1)}k`)

const DAY = new Intl.DateTimeFormat('en', { month: 'long', day: 'numeric', year: 'numeric' })

export const on = (at: number) => DAY.format(new Date(at))
