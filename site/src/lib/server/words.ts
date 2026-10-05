import { stemmer } from 'stemmer'

export const words = (asked: string) => asked.toLowerCase().split(/\s+/).filter(Boolean)

// A word cut to its root, so installation and install compare equal.
export const root = (word: string) => stemmer(word.toLowerCase())

// The roots of a text's words, kept per text since the pages never change while the Worker is up.
const rooted = new Map<string, Set<string>>()

export function roots(text: string) {
  const lower = text.toLowerCase()
  if (!rooted.has(lower)) rooted.set(lower, new Set((lower.match(/[a-z]+/g) ?? []).map((word) => root(word))))
  return rooted.get(lower)!
}

// Every word must match, in any order, as written or as another form of itself.
export const holds = (text: string, asked: string) => words(asked).every((word) => text.toLowerCase().includes(word) || roots(text).has(root(word)))

// Where the whole query or else its first word lands, written or in another form, so a snippet opens on it.
export function landing(text: string, asked: string): [number, number] {
  const lower = text.toLowerCase()
  const whole = lower.indexOf(asked.toLowerCase())
  if (whole >= 0) return [whole, asked.length]

  for (const word of words(asked)) {
    const at = lower.indexOf(word)
    if (at >= 0) return [at, word.length]
  }

  const wanted = new Set(words(asked).map((word) => root(word)))
  for (const found of lower.matchAll(/[a-z]+/g)) if (wanted.has(root(found[0]))) return [found.index, found[0].length]

  return [-1, 0]
}
