import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { DatabaseSync } from 'node:sqlite'
import { test } from 'node:test'
import { drift, stale } from './schema'
import { SITE_DIR } from './constants'

const SCHEMA = readFileSync(`${SITE_DIR}db/schema.sql`, 'utf8')

// What production answers for sqlite_master once these statements have run on it.
function production(...sql: string[]) {
  const db = new DatabaseSync(':memory:')
  for (const each of sql) db.exec(each)
  return db.prepare('select type, name, sql from sqlite_master where sql is not null').all() as { type: string; name: string; sql: string }[]
}

const pending = (sql: string) => [{ file: '0002_change.sql', sql }]
const withBio = SCHEMA.replace('  handle_changed_at integer,', '  handle_changed_at integer,\n  website text,')

test('a production built from the schema agrees with it', () => {
  assert.deepEqual(drift(production(SCHEMA), [], SCHEMA), [])
})

// An alter rewrites the stored text and appends the column, and neither may count.
test('an applied alter agrees with the schema that places the column anywhere', () => {
  assert.deepEqual(drift(production(SCHEMA, 'alter table user add column website text'), [], withBio), [])
})

test('a pending migration agrees with the schema it leads to', () => {
  assert.deepEqual(drift(production(SCHEMA), pending('alter table user add column website text'), withBio), [])
})

test('a column only in the schema asks for a migration', () => {
  const [issue, ...rest] = drift(production(SCHEMA), [], withBio)
  assert.match(issue!, /column user\.website .* Add a migration/)
  assert.deepEqual(rest, [])
})

test('a column only in a migration asks for the schema', () => {
  const [issue, ...rest] = drift(production(SCHEMA), pending('alter table user add column website text'), SCHEMA)
  assert.match(issue!, /column user\.website .* missing from schema\.sql/)
  assert.deepEqual(rest, [])
})

test('a check that changes its rule is caught on both sides', () => {
  const apple = SCHEMA.replace("('email', 'github', 'google')", "('email', 'github', 'google', 'apple')")
  const issues = drift(production(SCHEMA), [], apple)

  assert.equal(issues.length, 2)
  assert.ok(issues.some((each) => each.includes("'apple'") && each.includes('Add a migration')))
})

test('a check written another way is the same rule', () => {
  const reworded = SCHEMA.replace('check (length(name) > 0)', 'CHECK((LENGTH(name)>0))')
  assert.deepEqual(drift(production(SCHEMA), [], reworded), [])
})

test('a table that loses strict is caught', () => {
  const loose = SCHEMA.replace(') strict;', ');')
  assert.equal(drift(production(SCHEMA), [], loose)[0], 'table user is strict 1, without rowid 0 on production with its migrations, but strict 0, without rowid 0 in schema.sql. Align one with the other.')
})

test('a migration that cannot run on production says which one', () => {
  assert.match(drift(production(SCHEMA), pending('alter table user add column bio text'), SCHEMA)[0]!, /0002_change\.sql fails on production/)
})

test('a migration production already recorded is due for deletion', () => {
  assert.deepEqual(stale(['0002_a.sql', '0003_b.sql'], ['0001_schema.sql', '0002_a.sql']), ['0002_a.sql is already applied on production. Delete it from site/db/migrations.'])
})
