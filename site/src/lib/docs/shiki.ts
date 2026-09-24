/* How code is coloured, in one place, because the build highlights the site's own pages and the client highlights a package's once it has the grammar. Two copies of this drift the moment a theme changes. */
export const THEMES = { light: 'github-light', dark: 'github-dark' } as const

// A fence names a dialect the site owns, and the grammar behind it is the one that ships.
export const ALIAS: Record<string, string> = { 'edge-python': 'python', output: 'text' }

// Plain text has no grammar, so a block asking for it is escaped and left alone.
export const PLAIN = 'text'

export const shikiConfig = { themes: THEMES, defaultColor: false as const, langAlias: ALIAS }
