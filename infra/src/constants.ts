import { fileURLToPath } from 'node:url'
import { RESOURCE_HASH, ZONE } from './public'

export { RESOURCE_HASH, ZONE }

export const ENV = process.env.EDGE_ENV === 'prod' ? 'prod' : 'dev'

// Production keeps what people made, its rows and published packages, and dev keeps nothing between promotes.
export const keeps = (env: string) => env === 'prod'

// Every name one environment owns, exported so a test can hold dev and prod side by side.
export function names(env: string) {
  const worker = `${RESOURCE_HASH}-${env}`
  const site = env === 'prod' ? ZONE : `${env}.${ZONE}`

  return { worker, db: `${worker}-db`, bucket: `${worker}-cdn`, site, www: `www.${site}`, cdn: `cdn.${site}` }
}

const OWN = names(ENV)

export const WORKER = OWN.worker
export const DB_NAME = OWN.db
export const BUCKET = OWN.bucket

export const SITE_DOMAIN = OWN.site
export const WWW_DOMAIN = OWN.www
export const CDN_DOMAIN = OWN.cdn
export const SITE_URL = `https://${SITE_DOMAIN}`
export const CDN_URL = `https://${CDN_DOMAIN}`
export const EMAIL_FROM = `no-reply@${SITE_DOMAIN}`

// tmp is only a CDN, each CI run stages under its prefix and expires in a day.
export const TMP_BUCKET = `${RESOURCE_HASH}-tmp-cdn`
export const TMP_CDN_DOMAIN = `cdn.tmp.${ZONE}`
export const TMP_CDN_URL = `https://${TMP_CDN_DOMAIN}`
export const TMP_EXPIRY_SECONDS = 86400

export const REPO_DIR = fileURLToPath(new URL('../../', import.meta.url))
export const SITE_DIR = fileURLToPath(new URL('../../site/', import.meta.url))
