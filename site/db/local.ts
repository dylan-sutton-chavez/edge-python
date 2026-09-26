import { execFileSync } from 'node:child_process'
import { rmSync } from 'node:fs'
import { DB_NAME } from '../../infra/src/constants'

rmSync('.wrangler/state/v3/d1', { recursive: true, force: true })

// A local database is born from the schema, migrations are only for production.
execFileSync('npx', ['wrangler', 'd1', 'execute', DB_NAME, '--local', '--file', 'db/schema.sql'], { stdio: 'inherit' })
execFileSync('npx', ['wrangler', 'd1', 'execute', DB_NAME, '--local', '--file', 'db/seed.sql'], { stdio: 'inherit' })
