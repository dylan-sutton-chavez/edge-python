export const MODEL = '@cf/google/gemma-4-26b-a4b-it'

export const TICK_MS = 10_000
export const FETCH_LIMIT = 50
export const PASSAGES = 5

// Characters Discord takes in one message, and tokens an answer may spend, which stays well inside it.
export const MAX_MESSAGE = 2_000
export const MAX_ANSWER = 700

// What a tick may answer and what a day may cost, the only two numbers that bound every model call.
export const PER_TICK = 3
export const PER_DAY = 500

// What one answer reads back, from a conversation that goes quiet after half an hour and ends after a day.
export const TURNS = 6
export const MAX_QUESTION = 500
export const MAX_SAID = 300
export const SESSION_MS = 30 * 60_000
export const HISTORY_MS = 24 * 60 * 60_000

export const DISCORD_API = 'https://discord.com/api/v10'
