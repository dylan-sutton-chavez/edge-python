import type { MiddlewareNext } from 'astro'
import { env } from 'cloudflare:workers'

/* Everything still being built, named by the address that already reaches it. A path is a page of its own and a fragment is a surface inside one, which is the name a deep link already uses. */
const DRAFT = ['/plans', '/runs', '/settings#billing', '/settings#invoices']

// What a surface renders with, so a group, its tab, its icon and its panel all answer to one name.
const HOOKS = ['data-group', 'data-panel', 'data-view', 'data-bar-icon']

const pages = DRAFT.filter((each) => !each.includes('#'))
const parts = DRAFT.flatMap((each) => each.split('#')[1] ?? [])

const isPage = (path: string) => pages.some((each) => path === each || path.startsWith(`${each}/`))

/* Every draft surface out of a response, found by the addresses above rather than by a list of its own, so a link cannot outlive the page it points at. */
function stripped(response: Response) {
  const rewriter = new HTMLRewriter().on('a', {
    element: (link) => {
      if (isPage(link.getAttribute('href') ?? '')) link.remove()
    }
  })

  for (const hook of HOOKS) {
    rewriter.on(`[${hook}]`, {
      element: (part) => {
        if (parts.includes((part.getAttribute(hook) ?? '').toLowerCase())) part.remove()
      }
    })
  }

  return rewriter.transform(response)
}

/* Answers a draft page as a 404 and takes every draft surface out of the pages that stay. The binding is read per request, because bindings are not populated while a module is still loading. */
export async function drafted(url: URL, next: MiddlewareNext) {
  if (env.DRAFT) return next()

  // Styled like any other miss, so the address gives nothing away.
  if (isPage(url.pathname)) {
    const missing = await next('/404')
    return new Response(missing.body, { status: 404, headers: missing.headers })
  }

  const answer = await next()
  return answer.headers.get('content-type')?.includes('text/html') ? stripped(answer) : answer
}
