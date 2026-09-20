import { fileURLToPath } from 'node:url'

// Fixed forever and never tied to a human-facing name.
export const RESOURCE_HASH = '506cf1'

export const ENV = process.env.EDGE_ENV === 'prod' ? 'prod' : 'dev'
export const ZONE = 'edgepython.com'

// Every name one environment owns, exported so a test can hold dev and prod side by side.
export function names(env: string) {
  const worker = `${RESOURCE_HASH}-${env}`
  const site = env === 'prod' ? ZONE : `${env}.${ZONE}`

  return { worker, db: `${worker}-db`, bucket: `${worker}-cdn`, site, cdn: `cdn.${site}` }
}

const OWN = names(ENV)

export const WORKER = OWN.worker
export const DB_NAME = OWN.db
export const BUCKET = OWN.bucket

export const SITE_DOMAIN = OWN.site
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
