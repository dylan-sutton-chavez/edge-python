const HTML_ESCAPES: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;' }

export const escapeHtml = (text: string) => text.replace(/[&<>]/g, (char) => HTML_ESCAPES[char]!)

export type Highlight = {
  (code: string): string
  dispose(): void
}

type Core = { codeToHtml(code: string, options: { lang: string; themes: Record<'light' | 'dark', string>; defaultColor: false }): string }

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

export function createHighlight(onChange: () => void): Highlight {
  let core: Core | null = null
  let disposed = false

  load().then((loaded) => {
    if (disposed) return
    core = loaded
    onChange()
  })

  const highlight = ((code: string) => {
    if (!core) return escapeHtml(code)

    return core
      .codeToHtml(code, { lang: 'python', themes: { light: 'github-light', dark: 'github-dark' }, defaultColor: false })
      .replace(/^<pre[^>]*>\s*<code[^>]*>/, '')
      .replace(/<\/code>\s*<\/pre>\s*$/, '')
  }) as Highlight

  highlight.dispose = () => { disposed = true }

  return highlight
}
