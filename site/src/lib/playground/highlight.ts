import { ALIAS, PLAIN, THEMES } from '../docs/shiki'

const HTML_ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;' }

export const escapeHtml = (text: string) => text.replace(/[&<>]/g, (char) => HTML_ESCAPES[char]!)

export type Highlight = {
  (code: string, lang?: string): string
  dispose(): void
}

type Core = {
  codeToHtml(code: string, options: { lang: string; themes: typeof THEMES; defaultColor: false }): string
  getLoadedLanguages(): string[]
  loadLanguage(lang: unknown): Promise<void>
}

let loading: Promise<Core> | null = null

function load(): Promise<Core> {
  if (loading) return loading

  loading = Promise.all([import('shiki/core'), import('shiki/engine/javascript')]).then(
    ([{ createHighlighterCore }, { createJavaScriptRegexEngine }]) =>
      createHighlighterCore({
        themes: [import('shiki/themes/github-light.mjs'), import('shiki/themes/github-dark.mjs')],
        langs: [import('shiki/langs/python.mjs')],
        engine: createJavaScriptRegexEngine({ forgiving: true })
      }) as Promise<Core>
  )

  return loading
}

/* Brings in the grammar a block asks for, looked up in Shiki's own registry so a name out of a published page can only ever name a grammar that ships. */
async function grammar(core: Core, lang: string): Promise<boolean> {
  if (core.getLoadedLanguages().includes(lang)) return true

  const { bundledLanguages } = await import('shiki/langs')
  const found = (bundledLanguages as Record<string, unknown>)[lang]
  if (!found) return false

  await core.loadLanguage(found)
  return true
}

/* Paints code once the grammar is in, and escapes it until then, so a page reads before it colours and never blocks on a download. */
export function createHighlight(onChange: () => void): Highlight {
  let core: Core | null = null
  let disposed = false

  load().then((loaded) => {
    if (disposed) return
    core = loaded
    onChange()
  })

  const asked = new Set<string>()

  const highlight = ((code: string, lang = 'python') => {
    const named = ALIAS[lang] ?? lang

    if (!core || named === PLAIN) return escapeHtml(code)

    if (!core.getLoadedLanguages().includes(named)) {
      // One attempt per language, then the block stays plain rather than asking again on every keystroke.
      if (!asked.has(named)) {
        asked.add(named)
        grammar(core, named).then((got) => got && !disposed && onChange())
      }
      return escapeHtml(code)
    }

    return core
      .codeToHtml(code, { lang: named, themes: THEMES, defaultColor: false })
      .replace(/^<pre[^>]*>\s*<code[^>]*>/, '')
      .replace(/<\/code>\s*<\/pre>\s*$/, '')
  }) as Highlight

  highlight.dispose = () => { disposed = true }

  return highlight
}
