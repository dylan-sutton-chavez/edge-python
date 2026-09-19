import Cloudflare from 'cloudflare'

const POLICY_NAME = 'Allowed devs'

async function ensure_tag(client: Cloudflare, account_id: string, name: string) {
  for await (const tag of client.zeroTrust.access.tags.list({ account_id })) {
    if (tag.name === name) return tag
  }

  console.log(`Creating Access tag "${name}"...`)
  return client.zeroTrust.access.tags.create({ account_id, name })
}

async function sync_policy(client: Cloudflare, account_id: string, app_id: string, allowed_emails: string[]) {
  for await (const policy of client.zeroTrust.access.applications.policies.list(app_id, { account_id })) {
    console.log(`Deleting policy "${policy.name ?? policy.id}"...`)
    await client.zeroTrust.access.applications.policies.delete(policy.id!, { app_id, account_id })
  }

  console.log(`Creating policy "${POLICY_NAME}" for ${allowed_emails.length} email(s)...`)
  await client.zeroTrust.access.applications.policies.create(app_id, {
    account_id,
    name: POLICY_NAME,
    decision: 'allow',
    include: allowed_emails.map((email) => ({ email: { email } }))
  })
}

export async function ensure_access(client: Cloudflare, account_id: string, resource_hash: string, domain: string) {
  const allowed_emails = (process.env.CLOUDFLARE_ACCESS_EMAILS ?? '').split(',').map((email) => email.trim()).filter(Boolean)

  if (!allowed_emails.length) {
    throw new Error('CLOUDFLARE_ACCESS_EMAILS is empty. Set it before creating the Access application.')
  }

  await ensure_tag(client, account_id, resource_hash)

  const spec = { domain, self_hosted_domains: [domain], type: 'self_hosted' as const, name: domain, tags: [resource_hash], session_duration: '24h' }

  let found: string | null = null
  const stale: string[] = []

  for await (const app of client.zeroTrust.access.applications.list({ account_id })) {
    if ('domain' in app && app.domain === domain) found = app.id!
    else if ('tags' in app && app.tags?.includes(resource_hash)) stale.push(app.id!)
  }

  for (const id of stale) {
    console.log(`Deleting stale Access application "${id}"...`)
    await client.zeroTrust.access.applications.delete(id, { account_id })
  }

  if (found) {
    console.log(`Access application for "${domain}" already exists, syncing it...`)
    await client.zeroTrust.access.applications.update(found, { account_id, ...spec })
    await sync_policy(client, account_id, found, allowed_emails)
    return found
  }

  console.log(`Creating Access application for "${domain}"...`)
  const app = await client.zeroTrust.access.applications.create({ account_id, ...spec })

  await sync_policy(client, account_id, app.id!, allowed_emails)

  return app.id!
}
