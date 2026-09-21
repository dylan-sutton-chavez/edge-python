// The layout rules a page has to follow, mirrored in cli/src/docs.rs and locked by tests/cases/docs.json.
export const ORDER = /^\d+[-_]/

export function check(page: string, text: string) {
  const segments = page.split('/')
  if (segments.length > 2) {
    throw new Error(`'${page}' nests deeper than one folder, a section and its pages is all the renderer orders`)
  }
  if (!segments.every((segment) => ORDER.test(segment))) {
    throw new Error(`'${page}' needs a numeric prefix on every segment, like '01-reference/02-cli.mdx'`)
  }

  let open: string | null = null
  let closed: string | null = null
  let headings = 0

  for (const line of front(page, text).split('\n')) {
    const trimmed = line.trim()
    if (trimmed.startsWith('```')) {
      if (open !== null) {
        closed = open
        open = null
      } else {
        const lang = trimmed.slice(3).trim()
        if (lang === 'output' && closed !== 'edge-python') {
          throw new Error(`'${page}' has an output block that follows no edge-python block`)
        }
        open = lang
        closed = null
      }
      continue
    }
    if (open === null) {
      if (line.startsWith('# ')) headings++
      // Blank lines keep two fences adjacent, prose between them does not.
      if (trimmed) closed = null
    }
  }

  if (open !== null) throw new Error(`'${page}' leaves a code fence unterminated`)
  if (headings !== 1) throw new Error(`'${page}' has ${headings} top-level headings, the renderer needs exactly one`)
}

// The page past its frontmatter, which names the page and describes it, and closes.
function front(page: string, text: string) {
  const start = text.startsWith('---\n') ? 4 : text.startsWith('---\r\n') ? 5 : -1
  if (start < 0) {
    throw new Error(`'${page}' opens with no frontmatter, a page needs a title and a description`)
  }

  const rest = text.slice(start)
  let at = 0
  let named = false
  let described = false

  for (const line of rest.split('\n')) {
    const trimmed = line.trimEnd()
    if (trimmed === '---') {
      if (!named || !described) {
        throw new Error(`'${page}' needs both a title and a description in its frontmatter`)
      }
      return rest.slice(at + line.length + 1)
    }
    if (trimmed) {
      const colon = trimmed.indexOf(':')
      if (colon < 0) {
        throw new Error(`'${page}' has the frontmatter line '${trimmed}', which is no key and value`)
      }
      const key = trimmed.slice(0, colon).trim()
      if (!trimmed.slice(colon + 1).trim()) {
        throw new Error(`'${page}' leaves the frontmatter '${key}' empty`)
      }
      if (key === 'title') named = true
      if (key === 'description') described = true
    }
    at += line.length + 1
  }
  throw new Error(`'${page}' leaves its frontmatter unterminated`)
}
