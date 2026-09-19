import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { REPO_DIR } from './constants'

export const PARTS = ['compiler', 'std', 'js', 'cli'] as const
export type Part = (typeof PARTS)[number]

const STD = ['json', 're', 'math', 'struct']

function need(path: string, hint: string) {
  if (!existsSync(path)) throw new Error(`${path} is missing, ${hint} first.`)
  return path
}

function copy(from: string, to: string) {
  mkdirSync(dirname(to), { recursive: true })
  cpSync(from, to, { recursive: true })
}

// Each part lays one build's outputs out in the CDN layout, the tree promote ships.
const STAGERS: Record<Part, (out: string) => void> = {
  compiler(out) {
    const target = join(REPO_DIR, 'target/wasm32-unknown-unknown')
    copy(need(join(target, 'release/compiler.wasm'), 'run cargo wasm'), join(out, 'compiler.wasm'))
    // The CLI embeds the speed build, later jobs read it from tmp and promote skips it.
    if (existsSync(join(target, 'cli/compiler.wasm'))) copy(join(target, 'cli/compiler.wasm'), join(out, '_build/compiler-cli.wasm'))
  },
  std(out) {
    for (const name of STD) {
      const release = join(REPO_DIR, `std/${name}/target/wasm32-unknown-unknown/release`)
      // Keyword-named crates emit an edge_ prefixed artifact.
      const built = [`${name}.wasm`, `edge_${name}.wasm`].map((file) => join(release, file)).find((file) => existsSync(file))
      copy(need(built ?? join(release, `${name}.wasm`), `build std/${name}`), join(out, `std/${name}.wasm`))
    }
    copy(join(REPO_DIR, 'std/test/src/entry.py'), join(out, 'std/test.py'))
  },
  js(out) {
    copy(need(join(REPO_DIR, 'js/dist'), 'run tsc in js'), join(out, 'js/src'))
    const builtins = join(REPO_DIR, 'js/builtins')
    for (const cap of readdirSync(builtins)) {
      if (existsSync(join(builtins, cap, 'src'))) copy(join(builtins, cap, 'src'), join(out, 'js/builtins', cap))
    }
  },
  cli(out) {
    for (const script of ['install.sh', 'uninstall.sh']) copy(join(REPO_DIR, 'cli/setup', script), join(out, 'cli', script))
    for (const file of readdirSync(join(REPO_DIR, 'cli')).filter((name) => /^edge-.+\.tar\.gz$/.test(name))) copy(join(REPO_DIR, 'cli', file), join(out, 'cli', file))
  }
}

export function stage(out: string, parts: readonly Part[] = PARTS) {
  rmSync(out, { recursive: true, force: true })
  mkdirSync(out, { recursive: true })
  for (const part of parts) STAGERS[part](out)
  return out
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [out, ...parts] = process.argv.slice(2)
  if (!out) throw new Error('Pass the output directory and optional parts, for example "npm run stage -- ../_cdn std".')

  const unknown = parts.find((part) => !(PARTS as readonly string[]).includes(part))
  if (unknown) throw new Error(`Unknown part "${unknown}", pick from ${PARTS.join(', ')}.`)

  stage(resolve(out), parts.length ? (parts as Part[]) : PARTS)
  console.log(`Staged ${parts.length ? parts.join(', ') : 'every part'} into ${resolve(out)}.`)
}
