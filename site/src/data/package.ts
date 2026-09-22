import type { Avatar } from '../lib/account/avatar'

export type Host = 'cli' | 'web' | 'actor'

export type Block =
  | { kind: 'text'; value: string }
  | { kind: 'heading'; value: string }
  | { kind: 'code'; code: string; output: string }

export type Page = { id: string; data: { title: string }; blocks: Block[] }
export type Release = { version: string; published: string; size: string }

export type Detail = {
  name: string
  version: string
  description: string
  repository: string
  license: string
  downloads: string
  size: string
  digest: string
  hosts: Host[]
  author: { handle: string; avatar: Avatar }
  releases: Release[]
  pages: Page[]
}

// The artifact answers this, not the author, so the name of each host reads as prose.
export const HOSTS: Record<Host, string> = {
  cli: 'the CLI',
  web: 'a browser',
  actor: 'an actor pool'
}

// Stands in for the registry while a published package has nowhere to keep its pages.
export const details: Record<string, Detail> = {
  json: {
    name: 'json',
    version: '0.4.2',
    description: 'JSON parsing and serialization.',
    repository: 'https://github.com/dylan-sutton-chavez/edge-json',
    license: 'MIT',
    downloads: '1.2k',
    size: '4.2 KB',
    digest: '9f2c4e7a1b8d3f5069ac2e4b7d1f8a3c6e9b0d2f4a7c1e8b3d6f9a2c5e8b1d4f',
    hosts: ['cli', 'web', 'actor'],
    author: { handle: 'dylan', avatar: { icon: 12, palette: 'sky' } },
    releases: [
      { version: '0.4.2', published: 'September 18, 2026', size: '4.2 KB' },
      { version: '0.4.1', published: 'August 30, 2026', size: '4.2 KB' },
      { version: '0.3.0', published: 'July 11, 2026', size: '3.8 KB' }
    ],
    pages: [
      {
        id: '01-getting-started/01-introduction',
        data: { title: 'Introduction' },
        blocks: [
          { kind: 'text', value: 'Reads and writes JSON with the same two calls everywhere, in the browser, in the CLI, and inside an actor. Nothing here touches the network or the filesystem.' },
          { kind: 'code', code: 'from json import loads\n\nprint(loads(\'{"ok": true}\')["ok"])', output: 'True' },
          { kind: 'heading', value: 'What it covers' },
          { kind: 'text', value: 'Objects, arrays, strings, numbers, booleans and null. A number comes back as an int when it has no fraction and no exponent, and as a float otherwise.' }
        ]
      },
      {
        id: '01-getting-started/02-installation',
        data: { title: 'Installation' },
        blocks: [
          { kind: 'text', value: 'Add the package and the manifest keeps the digest of the version it found, so a build fails if those bytes ever change.' },
          { kind: 'code', code: 'from json import dumps\n\nprint(dumps({"pinned": True}))', output: '{"pinned": true}' }
        ]
      },
      {
        id: '02-reference/01-loads',
        data: { title: 'loads' },
        blocks: [
          { kind: 'text', value: 'Parses a string and raises ValueError on anything it cannot read, with the offset where it stopped.' },
          { kind: 'code', code: 'from json import loads\n\nprint(loads("[1, 2.5, null]"))', output: '[1, 2.5, None]' }
        ]
      },
      {
        id: '02-reference/02-dumps',
        data: { title: 'dumps' },
        blocks: [
          { kind: 'text', value: 'Serializes a value and keeps the insertion order of a dict, so the output is the same on every run.' },
          { kind: 'code', code: 'from json import dumps\n\nprint(dumps([1, "two", None]))', output: '[1, "two", null]' }
        ]
      }
    ]
  },
  dom: {
    name: 'dom',
    version: '0.9.0',
    description: 'Read and change the page from Edge Python.',
    repository: 'https://github.com/dylan-sutton-chavez/edge-dom',
    license: 'MIT',
    downloads: '840',
    size: '18.6 KB',
    digest: '3a71f0c8e2d45b96a1c7e30f8d2b64a95c7e1f03b8d64a27e91c5f08b3d72a64',
    hosts: ['web'],
    author: { handle: 'dylan', avatar: { icon: 12, palette: 'sky' } },
    releases: [
      { version: '0.9.0', published: 'September 2, 2026', size: '18.6 KB' },
      { version: '0.8.1', published: 'June 24, 2026', size: '17.9 KB' }
    ],
    pages: [
      {
        id: '01-getting-started/01-introduction',
        data: { title: 'Introduction' },
        blocks: [
          { kind: 'text', value: 'Reaches the page the program is running in, so it needs a browser. A CLI run has no document to reach and refuses the import.' },
          { kind: 'code', code: 'from dom import query\n\nquery("h1").text = "Hello"', output: '' }
        ]
      },
      {
        id: '02-reference/01-query',
        data: { title: 'query' },
        blocks: [
          { kind: 'text', value: 'Finds the first element matching a selector and returns None when nothing matches.' },
          { kind: 'code', code: 'from dom import query\n\nprint(query("#missing"))', output: 'None' }
        ]
      }
    ]
  }
}
