// Acts on control characters like a terminal, so `print('\r…')` progress lines overwrite instead of stacking.
export function applyControls(text: string) {
  if (!/[\r\b\t\f]/.test(text)) return text

  const lines: string[] = []
  let line = ''
  let column = 0

  const put = (char: string) => {
    line = line.slice(0, column) + char + line.slice(column + 1)
    column += 1
  }

  for (const char of text) {
    if (char === '\n' || char === '\f') {
      lines.push(line)
      line = ''
      column = 0
    } else if (char === '\r') {
      column = 0
    } else if (char === '\b') {
      if (column > 0) column -= 1
    } else if (char === '\t') {
      do put(' ')
      while (column % 4)
    } else {
      put(char)
    }
  }

  lines.push(line)

  return lines.join('\n')
}
