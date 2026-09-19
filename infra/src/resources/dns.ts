import { client } from '../client'

export async function has_record(zone_id: string, name: string, type: string) {
  for await (const _ of client.dns.records.list({ zone_id, name: { exact: name }, type: type as never })) return true
  return false
}
