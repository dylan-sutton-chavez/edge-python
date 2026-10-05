import type { TraceEvent } from '../../../../js/src/system/trace'
import { createHighlight } from '../playground/highlight'
import { createTrace } from '../playground/trace'
import { ease, seen, still, wait } from './motion'

// What the scan in the demo reports on a small Rails app, thirty rows in all.
const APP = [
  'controllers/application_controller.rb', 'controllers/files_controller.rb', 'controllers/orders_controller.rb', 'controllers/products_controller.rb',
  'controllers/search_controller.rb', 'controllers/sessions_controller.rb', 'controllers/users_controller.rb', 'helpers/application_helper.rb',
  'helpers/products_helper.rb', 'jobs/application_job.rb', 'jobs/import_job.rb', 'mailers/application_mailer.rb', 'mailers/order_mailer.rb',
  'models/application_record.rb', 'models/order.rb', 'models/product.rb', 'models/user.rb', 'views/layouts/application.html.erb',
  'views/orders/index.html.erb', 'views/orders/show.html.erb', 'views/products/index.html.erb', 'views/products/show.html.erb',
  'views/search/index.html.erb', 'views/sessions/new.html.erb', 'views/users/edit.html.erb'
]
const FOUND: Record<string, string> = { 'controllers/files_controller.rb': 'CWE-22', 'controllers/products_controller.rb': 'CWE-89', 'helpers/products_helper.rb': 'CWE-79', 'views/search/index.html.erb': 'CWE-79' }

// How long the code rests, how far apart rows land, and how long the trace stays.
const REST = 3000
const ROW = 140
const HOLD = 2000

/* The events a run of the scan leaves, timed as the host would, and its length. */
function scan(): [TraceEvent[], number] {
  const run: TraceEvent[] = [{ kind: 'call', at: 0, ms: 3, pkg: 'main', call: 'fs.list', scope: 'shop/app', outcome: 'ok' }]
  let took = 3
  APP.forEach((file, index) => {
    const ms = 0.4 + ((index * 7) % 3)
    run.push({ kind: 'call', at: took, ms, pkg: 'main', call: 'fs.read', scope: `shop/app/${file}`, outcome: 'ok' })
    took += ms
    if (FOUND[file]) run.push({ kind: 'print', at: took, text: `${FOUND[file]} shop/app/${file}` })
  })
  return [run, took]
}

/* The playground replayed in `demo`, the code and its trace rising over each other in turn. */
export function replay(demo: HTMLElement) {
  const [code, traced] = [...demo.querySelectorAll<HTMLElement>('[data-face]')] as [HTMLElement, HTMLElement]
  const progress = demo.querySelector<HTMLElement>('[data-progress]')!
  const scroller = traced.querySelector<HTMLElement>('[data-scroller]')!
  const trace = createTrace(traced.querySelector<HTMLElement>('[data-rows]')!)
  const [run, took] = scan()

  // Coloured by the same highlighter as the playground, once its grammar is in.
  const source = demo.querySelector<HTMLElement>('[data-lang]')!
  const text = source.textContent!
  const highlight = createHighlight(() => { source.innerHTML = highlight(text, source.dataset.lang) })
  source.innerHTML = highlight(text, source.dataset.lang)

  // Held to the newest row unless the reader scrolled up to an older one.
  const land = (event: TraceEvent) => {
    const pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 24
    trace.push(event)
    requestAnimationFrame(() => { if (pinned) scroller.scrollTop = scroller.scrollHeight })
  }

  const fill = (ms: number) => progress.animate([{ scale: '0 1', opacity: 1 }, { scale: '1 1', opacity: 1 }], { duration: ms, easing: 'linear', fill: 'forwards' }).finished

  const cover = async (incoming: HTMLElement) => {
    const outgoing = incoming === code ? traced : code
    outgoing.style.zIndex = '0'
    incoming.style.zIndex = '1'
    progress.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 200, fill: 'forwards' })
    incoming.querySelector('[data-rim]')!.animate([{ opacity: 1 }, { opacity: 1, offset: 0.8 }, { opacity: 0 }], { duration: 1300 })
    await incoming.animate([{ translate: '0 100%' }, { translate: '0 0' }], { duration: 700, easing: ease, fill: 'forwards' }).finished
    outgoing.getAnimations().forEach((each) => each.cancel())
    outgoing.style.translate = '0 100%'
  }

  const play = async () => {
    for (;;) {
      await fill(REST)
      trace.reset()
      scroller.scrollTop = 0
      await cover(traced)
      const rows = (async () => {
        for (const event of run) {
          land(event)
          await wait(ROW)
        }
        trace.finish(took)
      })()
      await fill(run.length * ROW + HOLD)
      await rows
      await cover(code)
    }
  }

  // A still page shows the run already done.
  if (still.matches) {
    traced.style.translate = '0 0'
    traced.style.zIndex = '1'
    run.forEach((event) => trace.push(event))
    trace.finish(took)
  } else {
    seen(demo, play)
  }
}
