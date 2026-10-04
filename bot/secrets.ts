import { execFileSync } from 'node:child_process'

const SECRETS = ['DISCORD_TOKEN']

// Pushed after the deploy, since a secret is written against a Worker that already exists.
const held = Object.fromEntries(SECRETS.map((name) => [name, process.env[name] ?? '']))
execFileSync('npx', ['wrangler', 'secret', 'bulk'], { input: JSON.stringify(held), stdio: ['pipe', 'inherit', 'inherit'] })
