import { client, zone_id } from '../client'
import { has_record } from './dns'

export async function ensure_email(zone: string, domain: string) {
  const id = await zone_id(zone)
  let found = null

  // The zone is shared with the docs and the CDN, so other sending domains are left alone.
  for await (const each of client.emailSending.subdomains.list({ zone_id: id })) {
    if (each.name === domain) found = each
  }

  if (found) console.log(`Email Sending on "${domain}" already enabled.`)
  else {
    console.log(`Enabling Email Sending on "${domain}"...`)
    found = await client.emailSending.subdomains.create({ zone_id: id, name: domain })
  }

  for await (const record of client.emailSending.subdomains.dns.get(found.tag, { zone_id: id })) {
    if (await has_record(id, record.name!, record.type!)) continue

    throw new Error(`Email Sending needs a ${record.type} record at "${record.name}". Cloudflare adds it when enabling Email Sending; add it from the dashboard if it is missing.`)
  }

  console.log(`Email Sending DNS for "${domain}" is in place.`)
}
