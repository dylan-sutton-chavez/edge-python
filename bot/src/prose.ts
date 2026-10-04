// Code is left as written, so an index like xs[1] is never read as a citation.
export const prose = (text: string, edit: (part: string) => string) =>
  text
    .split(/(```[\s\S]*?(?:```|$)|`[^`\n]*`)/)
    .map((part, at) => (at % 2 ? part : edit(part)))
    .join('')
