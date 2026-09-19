import { execFileSync } from 'node:child_process'
import { SITE_DIR } from '../constants'

const SECRETS = ['OAUTH_GITHUB_ID', 'OAUTH_GITHUB_SECRET', 'OAUTH_GOOGLE_ID', 'OAUTH_GOOGLE_SECRET']

// Builds and publishes the Worker, wrangler.json routes it to its custom domain.
export function deploy_site() {
  execFileSync('npm', ['run', 'deploy'], { cwd: SITE_DIR, stdio: 'inherit' })
}

export function push_secrets() {
  const secrets = Object.fromEntries(SECRETS.map((name) => [name, process.env[name] ?? '']))
  execFileSync('npx', ['wrangler', 'secret', 'bulk'], { cwd: SITE_DIR, input: JSON.stringify(secrets), stdio: ['pipe', 'inherit', 'inherit'] })
}
