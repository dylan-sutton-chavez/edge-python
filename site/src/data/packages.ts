import type { Avatar } from '../lib/account/avatar'

export type Package = {
  name: string
  description: string
  href: string
  author: { handle: string; avatar: Avatar }
  downloads: string
}

export const packages: Package[] = [
  {
    name: 'json',
    description: 'JSON parsing and serialization.',
    href: '/package/json',
    author: { handle: 'dylan', avatar: { icon: 12, palette: 'sky' } },
    downloads: '1.2k'
  }
]
