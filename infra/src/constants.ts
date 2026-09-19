import { fileURLToPath } from 'node:url'

// Fixed forever and never tied to a human-facing name.
export const RESOURCE_HASH = '506cf1'

export const ENV = 'dev'
export const ZONE = 'edgepython.com'

export const WORKER = `${RESOURCE_HASH}-${ENV}`
export const DB_NAME = `${WORKER}-db`
export const BUCKET = `${WORKER}-cdn`

export const SITE_DOMAIN = `${ENV}.${ZONE}`
export const CDN_DOMAIN = `cdn.${ENV}.${ZONE}`
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
