const MINUTE = 60_000

/* How long a mailed code stays good, which the routes, the mail and the dialogs all have to agree on. */
export const CODE_LIFE = 10 * MINUTE

// What the mail tells the reader, so the copy cannot drift from the expiry above.
export const CODE_MINUTES = CODE_LIFE / MINUTE

/* What a code may be spent on. An address holds one code, so asking for another purpose replaces it. */
export const PURPOSES = ['sign_in', 'change_email', 'delete_account'] as const

export type Purpose = (typeof PURPOSES)[number]
