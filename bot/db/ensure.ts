import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { BOT_DB } from '../names'

const SCHEMA = readFileSync(new URL('./schema.sql', import.meta.url), 'utf8')

// What wrangler, D1 and SQLite keep for themselves, never part of the schema.
const OWN = /^(_cf_|sqlite_|d1_)/

const wrangler = (args: string[], quiet = false) =>
  execFileSync('npx', ['wrangler', ...args], { encoding: 'utf8', stdio: quiet ? ['ignore', 'pipe', 'ignore'] : 'inherit' })

const sql = (command: string, quiet = false) => wrangler(['d1', 'execute', BOT_DB, '--remote', '--yes', '--json', '--command', command], quiet)

// Created once and never dropped, since a new database would change the id the Worker is bound to.
const held: { name: string }[] = JSON.parse(wrangler(['d1', 'list', '--json'], true))
if (!held.some((each) => each.name === BOT_DB)) wrangler(['d1', 'create', BOT_DB])

// Every statement says if not exists, so this adds what is new and keeps the rows of what is not.
wrangler(['d1', 'execute', BOT_DB, '--remote', '--yes', '--file=db/schema.sql'])

// A table the schema stopped declaring is dropped, so taking one out is an edit to the schema alone.
const declared = new Set([...SCHEMA.matchAll(/create table if not exists (\w+)/g)].map((each) => each[1]!))
const live = (JSON.parse(sql("select name from sqlite_master where type = 'table'", true)) as { results: { name: string }[] }[])
  .flatMap((each) => each.results)
  .map((each) => each.name)

const gone = live.filter((name) => !declared.has(name) && !OWN.test(name))

if (gone.length) {
  console.log(`dropping ${gone.join(', ')} from ${BOT_DB}, which the schema no longer declares`)
  sql(gone.map((name) => `drop table if exists "${name}"`).join('; '))
}
