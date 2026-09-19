import Cloudflare from 'cloudflare'

export const client = new Cloudflare({ apiToken: process.env.CLOUDFLARE_API_TOKEN })
export const account_id = process.env.CLOUDFLARE_ACCOUNT_ID!

export async function zone_id(name: string) {
  for await (const zone of client.zones.list({ name })) return zone.id

  throw new Error(`Zone "${name}" is not on this account.`)
}
