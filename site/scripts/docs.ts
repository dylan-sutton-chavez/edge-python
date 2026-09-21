import { readdirSync, readFileSync } from 'node:fs'
import { join, relative, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { check } from '../src/lib/docs/convention'

// The renderer has no way to report a malformed page, so a build refuses to start with one.
const DOCS = fileURLToPath(new URL('../../docs/', import.meta.url))

const walk = (dir: string): string[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
    entry.isDirectory() ? walk(join(dir, entry.name)) : entry.name.endsWith('.mdx') ? [join(dir, entry.name)] : []
  )

const pages = walk(DOCS).sort()
const failed: string[] = []

for (const file of pages) {
  const page = relative(DOCS, file).split(sep).join('/')
  try {
    check(page, readFileSync(file, 'utf8'))
  } catch (e) {
    failed.push(`  ${(e as Error).message}`)
  }
}

if (failed.length > 0) {
  console.error(`${failed.length} of ${pages.length} docs pages are off the convention`)
  console.error(failed.join('\n'))
  process.exit(1)
}

console.log(`${pages.length} docs pages follow the convention`)
