import { RESOURCE_HASH, ZONE } from '../infra/src/public'

// Only facts come from infra, since the bot has no environment and the first build that borrowed one pointed it at dev.
export const BOT = `${RESOURCE_HASH}-bot`
export const BOT_DB = `${BOT}-db`
export const ASK_DOMAIN = `ask.${ZONE}`

// Always the published site, since the bot answers one server and reads what everybody else reads.
export const SITE = `https://${ZONE}`

export const GUILD = '1556245444147806249'

// Anything but 1 ships the http side alone, the way npm run dev runs it.
export const DISCORD = '1'
