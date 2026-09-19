const PAIRS: Record<string, string> = { '(': ')', '[': ']', '{': '}', '"': '"', "'": "'" }
const OPENERS = new Set(Object.keys(PAIRS))
const CLOSERS = new Set(Object.values(PAIRS))
const STRING_START = /^([fFrRbBuU]{0,2})("""|'''|"|')/
const DEDENT = /^\s*(?:elif|else|except|finally|case)\b[^:]*:$/
const WHITESPACE = /[ \t]/

const TAB = 2
const MAX_LINES = 999

type Edit = {
  from: number
  to: number
  insert: string
  caret?: number
  select?: [number, number]
  pair?: boolean
}

type Line = { start: number; end: number; body: string; column: number }

const lineAt = (text: string, caret: number): Line => {
  const start = text.lastIndexOf('\n', caret - 1) + 1
  const next = text.indexOf('\n', caret)
  const end = next === -1 ? text.length : next

  return { start, end, body: text.slice(start, end), column: caret - start }
}

const indentOf = (text: string) => {
  for (let i = 0; i < text.length; i++) if (!WHITESPACE.test(text[i]!)) return i
  return text.length
}

const lineRange = (text: string, from: number, to: number) => {
  const start = text.lastIndexOf('\n', from - 1) + 1
  const anchor = to > from && text[to - 1] === '\n' ? to - 1 : to
  const next = text.indexOf('\n', anchor)

  return { start, end: next === -1 ? text.length : next }
}

const stringContext = (text: string, caret: number) => {
  let i = 0
  let quote = ''
  let formatted = false

  while (i < caret) {
    if (!quote) {
      if (text[i] === '#') {
        const next = text.indexOf('\n', i)
        if (next === -1 || next >= caret) return { inString: false, formatted: false }
        i = next + 1
        continue
      }

      const match = text.slice(i).match(STRING_START)
      if (match && i + match[0].length <= caret) {
        quote = match[2]!
        formatted = /[fF]/.test(match[1]!)
        i += match[0].length
        continue
      }

      i++
      continue
    }

    if (quote.length === 1 && text[i] === '\\') {
      i += 2
      continue
    }

    if (text.slice(i, i + quote.length) === quote) {
      i += quote.length
      quote = ''
      formatted = false
      continue
    }

    i++
  }

  return { inString: !!quote, formatted }
}

const dedentCommon = (text: string) => {
  const lines = text.split('\n')
  const filled = lines.filter((line) => line.trim().length > 0)
  if (!filled.length) return text

  const common = Math.min(...filled.map((line) => line.match(/^[ \t]*/)![0].length))

  return common ? lines.map((line) => line.slice(Math.min(line.length, common))).join('\n') : text
}

const unwrapFence = (text: string) => text.match(/^```python\n([\s\S]*?)\n```\s*$/)?.[1] ?? text

export const transitions = {
  character(text: string, caret: number, key: string): Edit | null {
    if (CLOSERS.has(key) && text[caret] === key) {
      return { from: caret, to: caret, insert: '', caret: caret + 1 }
    }

    if (!OPENERS.has(key)) return null

    const { inString, formatted } = stringContext(text, caret)
    if (inString && !(formatted && key === '{')) return null

    if ((key === '"' || key === "'") && text[caret - 2] === key && text[caret - 1] === key) {
      return { from: caret - 2, to: caret, insert: key.repeat(6), caret: caret + 1 }
    }

    return { from: caret, to: caret, insert: key + PAIRS[key], caret: caret + 1, pair: true }
  },

  wrap(text: string, from: number, to: number, key: string): Edit {
    return {
      from,
      to,
      insert: key + text.slice(from, to) + PAIRS[key],
      select: [from + 1, to + 1]
    }
  },

  backspace(text: string, caret: number, pairedAt: number): Edit | null {
    if (caret === 0) return null

    if (caret === pairedAt && PAIRS[text[caret - 1]!] === text[caret]) {
      return { from: caret - 1, to: caret + 1, insert: '', caret: caret - 1 }
    }

    if (!WHITESPACE.test(text[caret - 1]!)) return null

    const line = lineAt(text, caret)
    if (line.column === 0 || line.column > indentOf(line.body)) return null

    const back = line.column % TAB || TAB

    return { from: caret - back, to: caret, insert: '', caret: caret - back }
  },

  tab(text: string, caret: number): Edit {
    const pad = ' '.repeat(TAB - (lineAt(text, caret).column % TAB))

    return { from: caret, to: caret, insert: pad, caret: caret + pad.length }
  },

  untab(text: string, caret: number): Edit | null {
    const line = lineAt(text, caret)
    const indent = indentOf(line.body)
    if (indent === 0) return null

    const removed = indent - Math.floor((indent - 1) / TAB) * TAB

    return {
      from: line.start,
      to: line.start + removed,
      insert: '',
      caret: line.column >= removed ? caret - removed : line.start
    }
  },

  enter(text: string, caret: number): Edit {
    const line = lineAt(text, caret)
    const before = line.body.slice(0, line.column)
    const indent = before.match(/^[ \t]*/)![0]
    const deeper = /[:\[({][ \t]*$/.test(before) ? ' '.repeat(TAB) : ''
    const pad = indent + deeper

    const opener = before.replace(/[ \t]+$/, '').slice(-1)
    const split = ['[', '(', '{'].includes(opener) && PAIRS[opener] === text[caret]

    return {
      from: caret,
      to: caret,
      insert: split ? `\n${pad}\n${indent}` : `\n${pad}`,
      caret: caret + 1 + pad.length
    }
  },

  colon(text: string, caret: number): Edit | null {
    const line = lineAt(text, caret)
    const body = line.body.slice(0, line.column) + ':' + line.body.slice(line.column)
    if (!DEDENT.test(body)) return null

    const removed = Math.min(indentOf(body), TAB)

    return {
      from: line.start,
      to: caret,
      insert: body.slice(removed, line.column + 1),
      caret: line.start + line.column - removed + 1
    }
  },

  indent(text: string, from: number, to: number): Edit {
    const range = lineRange(text, from, to)
    const lines = text.slice(range.start, range.end).split('\n')
    const pad = ' '.repeat(TAB)

    return {
      from: range.start,
      to: range.end,
      insert: lines.map((line) => pad + line).join('\n'),
      select: [from + TAB, to + lines.length * TAB]
    }
  },

  outdent(text: string, from: number, to: number): Edit | null {
    const range = lineRange(text, from, to)
    const lines = text.slice(range.start, range.end).split('\n')

    let first = 0
    let total = 0

    const dedented = lines.map((line, index) => {
      const removed = Math.min(indentOf(line), TAB)
      total += removed
      if (index === 0) first = removed

      return line.slice(removed)
    })

    if (total === 0) return null

    return {
      from: range.start,
      to: range.end,
      insert: dedented.join('\n'),
      select: [Math.max(range.start, from - first), to - total]
    }
  }
}

export type EditorOptions = {
  input: HTMLTextAreaElement
  view: HTMLElement
  highlight: (code: string) => string
  onRun: (source: string) => void
  minLines?: number
  maxLines?: number
}

export function createEditor(options: EditorOptions) {
  const { input, view, highlight, onRun, minLines = 1, maxLines = 5 } = options
  const listeners = new AbortController()
  const { signal } = listeners

  let pairedAt = -1
  let composing = false

  const style = getComputedStyle(input)
  const lineHeight = parseFloat(style.lineHeight)

  // Monospace, so one probe gives the advance width every column shares.
  const measure = () => {
    const probe = document.createElement('span')
    probe.style.cssText = 'position:absolute;visibility:hidden;white-space:pre'
    probe.style.fontFamily = style.fontFamily
    probe.style.fontSize = style.fontSize
    probe.textContent = '0'.repeat(20)

    document.body.appendChild(probe)
    const width = probe.getBoundingClientRect().width / 20
    probe.remove()

    return width || 8
  }

  let columnWidth = measure()
  document.fonts.ready.then(() => { columnWidth = measure() })

  const resize = () => {
    const lines = input.value.split('\n').length
    input.style.height = `${Math.min(Math.max(lines, minLines), maxLines) * lineHeight}px`
  }

  const render = () => {
    const code = input.value
    view.innerHTML = highlight(code) + (code.endsWith('\n') ? ' ' : '')
    view.parentElement!.scrollTop = input.scrollTop
    view.parentElement!.scrollLeft = input.scrollLeft
  }

  // setSelectionRange moves the caret without scrolling to it, so past `maxLines` the caret walks off-screen silently.
  const reveal = () => {
    const caret = input.selectionEnd
    const before = input.value.slice(0, caret)
    const row = before.split('\n').length - 1
    const column = caret - (before.lastIndexOf('\n') + 1)

    const top = row * lineHeight
    if (top < input.scrollTop) input.scrollTop = top
    else if (top + lineHeight > input.scrollTop + input.clientHeight) {
      input.scrollTop = top + lineHeight - input.clientHeight
    }

    const left = column * columnWidth
    if (left < input.scrollLeft) input.scrollLeft = Math.max(0, left - columnWidth * 2)
    else if (left + columnWidth > input.scrollLeft + input.clientWidth) {
      input.scrollLeft = left + columnWidth * 2 - input.clientWidth
    }

    render()
  }

  const sync = () => {
    resize()
    render()
    reveal()
  }

  const place = (edit: Edit) => {
    const [from, to] = edit.select ?? [edit.caret ?? edit.from, edit.caret ?? edit.from]
    input.setSelectionRange(from, to)
  }

  const apply = (edit: Edit | null) => {
    if (!edit) return false

    if (edit.from !== edit.to || edit.insert) {
      input.setSelectionRange(edit.from, edit.to)

      // execCommand keeps the browser's native undo stack, which a direct `value =` write destroys.
      if (!document.execCommand('insertText', false, edit.insert)) {
        input.value = input.value.slice(0, edit.from) + edit.insert + input.value.slice(edit.to)
      }
    }

    place(edit)
    pairedAt = edit.pair ? (edit.caret ?? -1) : -1
    sync()

    return true
  }

  const insert = (raw: string) => {
    const text = unwrapFence(raw).replace(/\r\n?/g, '\n').replace(/\t/g, ' '.repeat(TAB))
    const clean = dedentCommon(text)
    const lines = input.value.split('\n').length + clean.split('\n').length - 1

    apply({
      from: input.selectionStart,
      to: input.selectionEnd,
      insert: lines > MAX_LINES ? clean.split('\n').slice(0, MAX_LINES - 1).join('\n') : clean,
      caret: input.selectionStart + clean.length
    })
  }

  input.addEventListener('compositionstart', () => { composing = true }, { signal })
  input.addEventListener('compositionend', () => { composing = false }, { signal })
  input.addEventListener('input', () => { pairedAt = -1; sync() }, { signal })
  input.addEventListener('keyup', reveal, { signal })
  input.addEventListener('click', render, { signal })
  input.addEventListener('scroll', render, { signal })

  input.addEventListener('keydown', (event) => {
    if (composing || event.isComposing) return

    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault()
      onRun(input.value)
      return
    }

    if (event.ctrlKey || event.metaKey || event.altKey) return

    const text = input.value
    const from = input.selectionStart
    const to = input.selectionEnd

    if (event.key === 'Enter' && text.split('\n').length >= MAX_LINES) {
      event.preventDefault()
      return
    }

    const edit =
      from !== to
        ? event.key === 'Tab' && event.shiftKey ? transitions.outdent(text, from, to)
        : event.key === 'Tab' ? transitions.indent(text, from, to)
        : OPENERS.has(event.key) ? transitions.wrap(text, from, to, event.key)
        : null
        : event.key === 'Backspace' ? transitions.backspace(text, from, pairedAt)
        : event.key === 'Enter' ? transitions.enter(text, from)
        : event.key === 'Tab' && event.shiftKey ? transitions.untab(text, from)
        : event.key === 'Tab' ? transitions.tab(text, from)
        : event.key === ':' ? transitions.colon(text, from)
        : OPENERS.has(event.key) || CLOSERS.has(event.key) ? transitions.character(text, from, event.key)
        : null

    if (apply(edit)) event.preventDefault()
  }, { signal })

  input.addEventListener('paste', (event) => {
    const raw = event.clipboardData?.getData('text')
    if (raw == null) return

    event.preventDefault()
    insert(raw)
  }, { signal })

  sync()

  return {
    getCode: () => input.value,
    setCode: (code: string) => { input.value = code; sync() },
    refresh: render,
    destroy: () => listeners.abort()
  }
}
