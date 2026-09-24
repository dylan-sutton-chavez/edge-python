import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { expect } from '@playwright/test'
import { test } from './helpers'
import { check } from '../src/lib/docs/convention'

type Case = { name: string; files: Record<string, string>; error?: string }

// The corpus cli/src/docs.rs runs, so neither side can change a rule on its own.
const CORPUS = fileURLToPath(new URL('../../tests/cases/docs.json', import.meta.url))
const cases: Case[] = JSON.parse(readFileSync(CORPUS, 'utf8'))

test('the corpus keeps cases on both sides of the rules', () => {
  expect(cases.length).toBeGreaterThan(0)
  expect(cases.some((each) => each.error)).toBe(true)
  expect(cases.some((each) => !each.error)).toBe(true)
})

for (const each of cases) {
  const pages = Object.keys(each.files)
    .filter((path) => path.endsWith('.mdx'))
    .sort()

  // A case with no page tests a rule about the directory itself, which only the CLI can see.
  if (pages.length === 0) continue

  test(each.name, () => {
    const run = () => {
      for (const page of pages) check(page, each.files[page]!)
    }
    if (each.error) expect(run).toThrow(each.error)
    else expect(run).not.toThrow()
  })
}
