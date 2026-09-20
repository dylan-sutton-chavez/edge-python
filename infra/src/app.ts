import { pathToFileURL } from 'node:url'
import { client, account_id } from './client'
import { ensure_access } from './resources/access'
import { ensure_dev_cdn, ensure_tmp_cdn } from './resources/cdn'
import { ensure_database } from './resources/database'
import { ensure_email } from './resources/email'
import { ensure_routing } from './resources/routing'
import { CONTACT_EMAIL, CONTACT_FORWARD_TO, RESOURCE_HASH, SITE_DOMAIN, ZONE } from './constants'

export async function ensure() {
  await ensure_database()
  await ensure_email(ZONE, SITE_DOMAIN)
  await ensure_routing(ZONE, CONTACT_EMAIL, CONTACT_FORWARD_TO)
  await ensure_access(client, account_id, RESOURCE_HASH, SITE_DOMAIN)
  await ensure_dev_cdn()
  await ensure_tmp_cdn()
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  ensure().catch((error) => {
    console.error(error)
    process.exit(1)
  })
}
