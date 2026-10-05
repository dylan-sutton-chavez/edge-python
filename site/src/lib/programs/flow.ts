type Point = { x: number; y: number }
type Path = { points: Point[]; lengths: number[]; total: number }
type Dot = { path: Path; phase: number; blue: boolean }

// Lines on each side of the bar, and where they leave the screen edge as a share of its height.
const LINES = 8
const TOP = 0.13
const BOTTOM = 0.92
// How far apart the lines arrive at the bar, as a share of its height.
const ARRIVE = 0.09
// Squares on each line, and one in this many drawn blue.
const DOTS = 3
const BLUE = 4
// What a square travels each second as a share of the screen width, measured on the original.
const SPEED = 0.0317
// How many times faster the squares run while the bar is lit, and the seconds they ease up and back down over.
const BOOST = 15.6
const RAMP = 1.5
const FALL = 1.2
const SIZE = 4.6
// Lines wider than a hairline, in step with the squares.
const WIDTH = 1.53
// How far a square fades in from the edge and out into the bar, so none of them pops.
const FADE = 24
const SAMPLES = 48

/* A cubic with level ends, so a line leaves the edge flat and meets the bar flat, measured along its length. */
function curve(from: Point, to: Point): Path {
  const mid = (from.x + to.x) / 2
  const points: Point[] = []
  for (let step = 0; step <= SAMPLES; step++) {
    const t = step / SAMPLES
    const u = 1 - t
    points.push({ x: u * u * u * from.x + 3 * u * t * mid + t * t * t * to.x, y: (u * u * u + 3 * u * u * t) * from.y + (3 * u * t * t + t * t * t) * to.y })
  }
  const lengths = [0]
  for (let step = 1; step < points.length; step++) lengths.push(lengths[step - 1]! + Math.hypot(points[step]!.x - points[step - 1]!.x, points[step]!.y - points[step - 1]!.y))
  return { points, lengths, total: lengths.at(-1)! }
}

/* Two colours of the theme met halfway, so the plain squares are the blue softened toward the page. */
function between(one: string, two: string): string {
  const channels = (hex: string) => [0, 2, 4].map((at) => parseInt(hex.trim().replace('#', '').slice(at, at + 2), 16))
  const [a, b] = [channels(one), channels(two)]
  return `rgb(${a.map((value, at) => Math.round((value + b[at]!) / 2)).join(', ')})`
}

/* The point a given distance along a path. */
function along(path: Path, distance: number): Point {
  let step = 1
  while (step < path.lengths.length - 1 && path.lengths[step]! < distance) step++
  const [start, end] = [path.lengths[step - 1]!, path.lengths[step]!]
  const t = end > start ? (distance - start) / (end - start) : 0
  const [a, b] = [path.points[step - 1]!, path.points[step]!]
  return { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t }
}

/* Draws lines from both screen edges into `target` on `canvas`, with squares travelling them toward it. */
export function flow(canvas: HTMLCanvasElement, target: HTMLElement) {
  const context = canvas.getContext('2d')!
  const still = matchMedia('(prefers-reduced-motion: reduce)')
  const field = target.querySelector('input')
  let paths: Path[] = []
  let dots: Dot[] = []
  let colors = { line: '', dot: '', blue: '' }
  // Measured from the canvas and not the window, since the canvas scrolls away with the first screen.
  let box = new DOMRect()

  const read = () => {
    const style = getComputedStyle(document.documentElement)
    colors = { line: style.getPropertyValue('--line'), dot: between(style.getPropertyValue('--link'), style.getPropertyValue('--page')), blue: style.getPropertyValue('--link') }
  }

  const layout = () => {
    const ratio = devicePixelRatio
    box = canvas.getBoundingClientRect()
    canvas.width = box.width * ratio
    canvas.height = box.height * ratio
    context.setTransform(ratio, 0, 0, ratio, 0, 0)

    const bar = target.getBoundingClientRect()
    const middle = bar.top - box.top + bar.height / 2
    const radius = bar.height / 2
    // How far in from the end of the bar its rounded edge sits at a given height, so a line meets the curve and not the box.
    const inset = (rise: number) => radius - Math.sqrt(Math.max(0, radius * radius - rise * rise))
    paths = []
    for (const [edge, side, inward] of [[0, bar.left - box.left, 1], [box.width, bar.right - box.left, -1]] as const) {
      for (let line = 0; line < LINES; line++) {
        const from = { x: edge, y: box.height * (TOP + ((BOTTOM - TOP) * line) / (LINES - 1)) }
        const rise = (line - (LINES - 1) / 2) * bar.height * ARRIVE
        const to = { x: side + inward * inset(rise), y: middle + rise }
        paths.push(curve(from, to))
      }
    }
    // Spaced along each line and nudged apart, so the squares never march in step.
    dots = paths.flatMap((path, line) => Array.from({ length: DOTS }, (_, at) => ({ path, phase: (at + ((line * 7) % 5) / 10) / DOTS, blue: (line * DOTS + at) % BLUE === 0 })))
  }

  const draw = (travel: number) => {
    context.clearRect(0, 0, box.width, box.height)
    context.lineWidth = WIDTH
    context.strokeStyle = colors.line
    for (const path of paths) {
      context.beginPath()
      path.points.forEach((point, step) => (step ? context.lineTo(point.x, point.y) : context.moveTo(point.x, point.y)))
      context.stroke()
    }

    const ratio = devicePixelRatio
    for (const dot of dots) {
      const distance = (travel + dot.phase * dot.path.total) % dot.path.total
      const point = along(dot.path, distance)
      context.globalAlpha = Math.min(1, distance / FADE, (dot.path.total - distance) / FADE)
      context.fillStyle = dot.blue ? colors.blue : colors.dot
      // Snapped to the device pixels, so a square stays sharp at any speed.
      context.fillRect(Math.round((point.x - SIZE / 2) * ratio) / ratio, Math.round((point.y - SIZE / 2) * ratio) / ratio, SIZE, SIZE)
    }
    context.globalAlpha = 1
  }

  // How far the squares have run, summed frame by frame since their speed changes.
  let travel = 0
  let rise = 0
  let last = 0

  const frame = (now: number) => {
    const seconds = last ? Math.min(0.1, (now - last) / 1000) : 0
    last = now
    // Fast while the pointer rests on the bar, while it holds the focus or while it holds a repo, and eased so the squares never jump.
    const lit = target.matches(':hover, :focus-within') || Boolean(field?.value)
    rise = Math.min(1, Math.max(0, rise + (lit ? seconds / RAMP : -seconds / FALL)))
    travel += seconds * SPEED * box.width * (1 + (BOOST - 1) * rise * rise * (3 - 2 * rise))
    // Hidden below 1024px, where the loop keeps its place but draws nothing. A still page runs this once, at no distance.
    if (canvas.clientWidth > 0) draw(travel)
    if (!still.matches) requestAnimationFrame(frame)
  }

  // A still page draws once each time the layout moves, a moving one on every frame anyway.
  const refit = () => {
    layout()
    if (still.matches && canvas.clientWidth > 0) draw(0)
  }

  read()
  layout()
  new MutationObserver(() => {
    read()
    if (still.matches) refit()
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] })
  addEventListener('resize', refit)
  document.fonts.ready.then(refit)
  requestAnimationFrame(frame)
}
