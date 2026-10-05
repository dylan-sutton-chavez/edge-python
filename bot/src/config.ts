export const MODEL = '@cf/google/gemma-4-26b-a4b-it'

// How long the published reference is kept before it is read again, so a release reaches the bot without a deploy.
export const REFERENCE_MS = 10 * 60_000

export const TICK_MS = 10_000
export const FETCH_LIMIT = 50

// How far back a place the bot sees for the first time is answered, so the question that opened it is not lost and its history is not replayed.
export const FIRST_MS = 5 * 60_000

// How many hits a search reads and how much of each it keeps.
export const PASSAGES = 5
export const MAX_PASSAGE = 1_500

// Characters Discord takes in one message, and tokens an answer may spend, which stays well inside it.
export const MAX_MESSAGE = 2_000
export const MAX_ANSWER = 700

// What a program the model runs may hold, spend and print, enough for a sum and never enough to stall a reply.
export const RUN_MEMORY = 16 << 20
export const RUN_OPS = 1_000_000
export const MAX_OUTPUT = 2_000
export const ROUNDS = 3

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
