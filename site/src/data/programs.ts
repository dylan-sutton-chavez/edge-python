import { OWNER } from '../lib/account/handle'

// Newest first, each one a page of its own under /program.
export type Program = { slug: string; name: string; description: string; author: string }

export const programs: Program[] = [
  { slug: 'edge-and-rails', name: 'Edge and rails', description: 'Paste any git repo and follow its code graph to every vulnerability it can reach.', author: OWNER }
]
