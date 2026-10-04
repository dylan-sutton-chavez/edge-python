const CODE = /```[\s\S]*?(?:```|$)|`[^`\n]*`/g
const PLACE = /(\d+)/g

// Code is left as written, so an index like xs[1] is never read as a citation.
export const prose = (text: string, edit: (part: string) => string) =>
  text
    .split(/(```[\s\S]*?(?:```|$)|`[^`\n]*`)/)
    .map((part, at) => (at % 2 ? part : edit(part)))
    .join('')

// Code held aside as placeholders, so a sentence quoting code stays one sentence and the code comes back as written.
export function held(text: string) {
  const code: string[] = []
  const hidden = text.replace(CODE, (each) => `${code.push(each) - 1}`)

  return {
    hidden,
    shown: (edited: string) => edited.replace(PLACE, (_, at: string) => code[Number(at)]!),
    uncoded: (edited: string) => edited.replace(PLACE, '')
  }
}
