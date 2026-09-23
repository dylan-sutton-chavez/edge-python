// Leading bytes marking an edge package, checked before a single field is read out of one.
const MAGIC = 'EDGEPKG\u0001'

// The same caps the CLI packs under, so a hostile bundle cannot walk the decoder off the end.
const MAX_FILES = 4096
const MAX_TOTAL = 64 << 20

// The prefix the CLI packs documentation under, which no import can reach.
const DOCS = '@docs/'

const text = new TextDecoder()

/* Everything the registry needs about a release, read out of the artifact rather than taken on the publisher's word. The values stay unknown because naming them is not the same as vouching for them, the route still validates every one. */
export type Packed = {
  name: unknown
  version: unknown
  description: unknown
  repository: unknown
  notice: string | null
  docs: Record<string, string>
}

/* Reads a `.edge` the way the CLI wrote it, a flat length-prefixed archive with no compression, so the whole format is a magic string and a loop. */
export function decode(artifact: Uint8Array): Map<string, Uint8Array> {
  const read = reader(artifact)

  if (text.decode(read.take(MAGIC.length)) !== MAGIC) throw new Error('That is not an edge package.')

  // The entry point, which the registry does not need but has to step over to reach the files.
  relative(read.string())

  const count = read.length()
  if (count > MAX_FILES) throw new Error(`A package carries ${MAX_FILES} files at most.`)

  const files = new Map<string, Uint8Array>()
  let total = 0

  for (let at = 0; at < count; at++) {
    const path = relative(read.string())
    const bytes = read.bytes()

    total += bytes.length
    if (total > MAX_TOTAL) throw new Error(`A package unpacks to ${MAX_TOTAL} bytes at most.`)

    files.set(path, bytes)
  }

  return files
}

/* What the bundle says about itself. A field the artifact does not carry comes back null, so an old CLI publishes with less rather than failing. */
export function packed(artifact: Uint8Array): Packed {
  const files = decode(artifact)

  const declared = files.get('edge.json')
  if (!declared) throw new Error('That package carries no edge.json, so it has nothing to publish under.')

  let manifest: Record<string, unknown>
  try {
    manifest = JSON.parse(text.decode(declared))
  } catch {
    throw new Error('The edge.json inside that package is not JSON.')
  }

  if (manifest == null || typeof manifest !== 'object' || Array.isArray(manifest)) {
    throw new Error('The edge.json inside that package is not an object.')
  }

  return {
    name: manifest.name,
    version: manifest.version,
    description: manifest.description ?? null,
    repository: manifest.repository ?? null,
    notice: notice(files),
    docs: docs(files)
  }
}

/* The LICENSE at the root whatever its extension, which the registry reads to name the license instead of believing a name. */
function notice(files: Map<string, Uint8Array>): string | null {
  for (const [path, bytes] of files) {
    if (path.includes('/')) continue
    if (path.split('.')[0]?.toUpperCase() === 'LICENSE') return text.decode(bytes)
  }

  return null
}

/* The pages the bundle carries, keyed by the path the site orders them with. */
function docs(files: Map<string, Uint8Array>): Record<string, string> {
  const found: Record<string, string> = {}

  for (const [path, bytes] of files) {
    if (path.startsWith(DOCS)) found[path.slice(DOCS.length)] = text.decode(bytes)
  }

  return found
}

/* Rejects any path that is absolute or climbs out with `..`, the whole anti zip-slip guard, since these paths become keys a page reads back. */
function relative(path: string): string {
  if (!path) throw new Error('That package has an empty path in it.')

  const segments = path.split('/').filter((segment) => segment !== '.')
  if (segments.some((segment) => segment === '' || segment === '..' || segment.includes('\\'))) {
    throw new Error(`'${path}' is not a plain relative path.`)
  }

  return segments.join('/')
}

/* A cursor over the archive, where every length is ascii digits closed by a newline. */
function reader(buf: Uint8Array) {
  let at = 0

  const take = (n: number) => {
    if (n < 0 || at + n > buf.length) throw new Error('That package is truncated.')
    return buf.subarray(at, (at += n))
  }

  const length = () => {
    const start = at
    while (at < buf.length && buf[at] !== 0x0a) at++
    if (at >= buf.length) throw new Error('That package is truncated.')

    const digits = text.decode(buf.subarray(start, at++))
    if (!/^\d+$/.test(digits)) throw new Error('That package has a length that is not a number.')

    return Number(digits)
  }

  const bytes = () => take(length())

  return { take, length, bytes, string: () => text.decode(bytes()) }
}
