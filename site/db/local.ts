import { execFileSync } from 'node:child_process'
import { rmSync } from 'node:fs'
import { DB_NAME } from '../../infra/src/constants'

rmSync('.wrangler/state/v3/d1', { recursive: true, force: true })

execFileSync('npx', ['wrangler', 'd1', 'migrations', 'apply', DB_NAME, '--local'], { stdio: 'inherit' })
execFileSync('npx', ['wrangler', 'd1', 'execute', DB_NAME, '--local', '--file', 'db/seed.sql'], { stdio: 'inherit' })
