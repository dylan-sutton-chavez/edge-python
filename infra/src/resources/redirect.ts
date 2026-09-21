import { client, zone_id } from '../client'
import { has_record } from './dns'

const PHASE = 'http_request_dynamic_redirect'

// The proxy answers before any origin, so the address only has to resolve, never to serve.
const PLACEHOLDER = '100::'

/* Sends one hostname to another at the edge, without waking the Worker to say so. */
export async function ensure_redirect(zone: string, from: string, to: string) {
  const id = await zone_id(zone)

  const resolves = (await has_record(id, from, 'AAAA')) || (await has_record(id, from, 'A'))

  if (resolves) console.log(`"${from}" already resolves.`)
  else {
    console.log(`Pointing "${from}" at the proxy...`)
    await client.dns.records.create({ zone_id: id, name: from, type: 'AAAA', content: PLACEHOLDER, proxied: true, ttl: 1 })
  }

  const expression = `http.host eq "${from}"`

  const rule = {
    action: 'redirect' as const,
    expression,
    description: `${from} to ${to}`,
    enabled: true,
    action_parameters: {
      from_value: {
        target_url: { expression: `concat("https://${to}", http.request.uri.path)` },
        status_code: 301 as const,
        preserve_query_string: true
      }
    }
  }

  // Rulesets are their own permission, so say which one rather than leak a bare 403.
  try {
    let ruleset = null
    for await (const each of client.rulesets.list({ zone_id: id })) {
      if (each.phase === PHASE && each.kind === 'zone') ruleset = each
    }

    if (!ruleset) {
      console.log(`Creating the redirect ruleset on "${zone}"...`)
      await client.rulesets.create({ zone_id: id, kind: 'zone', name: 'Redirects', phase: PHASE, rules: [rule] })
      return
    }

    const current = await client.rulesets.get(ruleset.id, { zone_id: id })
    if (current.rules?.some((each) => each.expression === expression)) {
      console.log(`Redirect for "${from}" already in place.`)
      return
    }

    console.log(`Redirecting "${from}" to "${to}"...`)
    await client.rulesets.rules.create(ruleset.id, { zone_id: id, ...rule })
  } catch (error) {
    const denied = (error as { status?: number }).status === 403
    throw new Error(denied
      ? `The API token cannot manage redirect rules on "${zone}". Give it the zone permission that covers dynamic redirects. ${error}`
      : `Redirecting "${from}" to "${to}" failed. ${error}`)
  }
}
