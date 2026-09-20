import { pathToFileURL } from 'node:url'
import { client, account_id } from './client'
import { ensure_access } from './resources/access'
import { ensure_site_cdn, ensure_tmp_cdn } from './resources/cdn'
import { ensure_database } from './resources/database'
import { ensure_email } from './resources/email'
import { ENV, RESOURCE_HASH, SITE_DOMAIN, ZONE } from './constants'

export async function ensure() {
  await ensure_database()
  await ensure_email(ZONE, SITE_DOMAIN)
  // Production is public, the gate exists so an unfinished dev is not.
  if (ENV === 'dev') await ensure_access(client, account_id, RESOURCE_HASH, SITE_DOMAIN)
  await ensure_site_cdn()
  await ensure_tmp_cdn()
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  ensure().catch((error) => {
    console.error(error)
    process.exit(1)
  })
}
