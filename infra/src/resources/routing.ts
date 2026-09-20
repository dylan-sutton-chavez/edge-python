import { client, account_id, zone_id } from '../client'

// Inbound mail, unrelated to Email Sending. One is how the site writes, this is how a person is reached.
export async function ensure_routing(zone: string, from: string, to: string) {
  const id = await zone_id(zone)

  let settings = await client.emailRouting.get({ zone_id: id })

  if (settings.enabled) console.log(`Email Routing on "${zone}" already enabled.`)
  else {
    console.log(`Enabling Email Routing on "${zone}"...`)
    settings = await client.emailRouting.enable({ zone_id: id, body: {} })
  }

  // Cloudflare adds the MX and TXT records itself, this reports when the zone did not end up ready.
  if (settings.status && settings.status !== 'ready') {
    throw new Error(`Email Routing on "${zone}" is "${settings.status}". Check its MX and TXT records in the dashboard.`)
  }

  let destination = null
  for await (const address of client.emailRouting.addresses.list({ account_id })) {
    if (address.email === to) destination = address
  }

  if (!destination) {
    console.log(`Adding "${to}" as a destination address...`)
    destination = await client.emailRouting.addresses.create({ account_id, email: to })
  }

  // Cloudflare will not forward anywhere the owner has not confirmed by clicking a link.
  if (!destination.verified) {
    throw new Error(`"${to}" is not verified yet. Cloudflare sent it a confirmation email, open it and click the link, then run this again.`)
  }

  for await (const rule of client.emailRouting.rules.list({ zone_id: id })) {
    const matches = rule.matchers?.some((matcher) => 'value' in matcher && matcher.value === from)
    if (matches) {
      console.log(`Routing for "${from}" already in place.`)
      return
    }
  }

  console.log(`Routing "${from}" to "${to}"...`)
  await client.emailRouting.rules.create({
    zone_id: id,
    actions: [{ type: 'forward', value: [to] }],
    matchers: [{ field: 'to', type: 'literal', value: from }],
    enabled: true,
    name: `${from} to ${to}`
  })
}
