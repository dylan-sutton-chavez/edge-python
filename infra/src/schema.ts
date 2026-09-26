import { readFileSync } from 'node:fs'
import { DatabaseSync } from 'node:sqlite'
import { pathToFileURL } from 'node:url'
import { parse, type Node } from 'sql-parser-cst'
import { find_database, migration, migrations, query, SCHEMA } from './resources/database'
import { DB_NAME } from './constants'

type Entry = { type: string; name: string; sql: string }

// What wrangler, D1 and SQLite keep for themselves, never part of the schema.
const INTERNAL = /^(sqlite_|_cf_|d1_migrations$)/

// Tables before what hangs off them, so a replay never indexes a table that is not there yet.
const ORDER = ['table', 'index', 'trigger', 'view']

// Source text and positions dropped, names folded as SQLite folds them, and parentheses that change nothing unwrapped.
function shape(node: unknown): unknown {
  if (Array.isArray(node)) return node.map(shape)
  if (!node || typeof node !== 'object') return node

  const { type, expr, name } = node as { type?: string; expr?: unknown; name?: unknown }
  if (type === 'paren_expr') return shape(expr)
  if (type === 'identifier' && typeof name === 'string') return { type, name: name.toLowerCase() }

  return Object.fromEntries(Object.entries(node).filter(([key]) => key !== 'text' && key !== 'range').map(([key, value]) => [key, shape(value)]))
}

// Every check of one table, keyed by its shape and shown by its own text.
function checks(sql: string) {
  const found = new Map<string, string>()

  JSON.stringify(parse(sql, { dialect: 'sqlite', includeRange: true }), (_, value: Node) => {
    if (value?.type === 'constraint_check' && value.range) found.set(JSON.stringify(shape(value)), sql.slice(...value.range))
    return value
  })

  return found
}

/* Every fact SQLite reports about a database, one line each, so two databases compare line by line. Columns come from pragmas because SQLite rewrites the stored text on every alter, and checks from their parsed shape because no pragma holds them. */
export function facts(db: DatabaseSync) {
  const out = new Map<string, string>()
  const all = <T>(sql: string, ...args: string[]) => db.prepare(sql).all(...args) as T[]

  const tables = all<{ name: string; type: string; strict: number; wr: number }>("select name, type, strict, wr from pragma_table_list where schema = 'main' and type in ('table', 'virtual')")

  for (const table of tables.filter(({ name }) => !INTERNAL.test(name))) {
    const { name } = table
    const sql = all<{ sql: string }>('select sql from sqlite_master where name = ?', name)[0]!.sql

    out.set(`table ${name}`, table.type === 'virtual' ? `virtual ${JSON.stringify(shape(parse(sql, { dialect: 'sqlite' })))}` : `strict ${table.strict}, without rowid ${table.wr}`)

    for (const column of all<{ name: string; type: string; notnull: number; dflt_value: string | null; pk: number }>('select * from pragma_table_xinfo(?)', name)) {
      out.set(`column ${name}.${column.name}`, `${column.type || 'untyped'}${column.notnull ? ' not null' : ''}${column.pk ? ` primary key ${column.pk}` : ''}${column.dflt_value === null ? '' : ` default ${column.dflt_value}`}`)
    }

    for (const index of all<{ name: string; unique: number; origin: string; partial: number }>('select * from pragma_index_list(?)', name)) {
      const columns = all<{ name: string }>('select name from pragma_index_info(?) order by seqno', index.name).map((each) => each.name).join(', ')
      // SQLite numbers the indexes it makes itself, so those are named by what they cover.
      const label = index.origin === 'c' ? index.name : `${index.origin === 'pk' ? 'primary key' : 'unique'} (${columns})`
      out.set(`index ${name}.${label}`, `${index.unique ? 'unique ' : ''}(${columns})${index.partial ? ' partial' : ''}`)
    }

    for (const key of all<{ id: number; table: string; from: string; to: string; on_update: string; on_delete: string }>('select * from pragma_foreign_key_list(?)', name)) {
      out.set(`foreign key ${name}.${key.from}`, `references ${key.table}(${key.to}) on update ${key.on_update} on delete ${key.on_delete}`)
    }

    // Filed under its shape and holding its text, so only the rule counts and the message can quote it.
    for (const [key, text] of checks(sql)) out.set(`check ${name} ${key}`, text)
  }

  return out
}

const built = (sql: string[]) => {
  const db = new DatabaseSync(':memory:')
  for (const each of sql) db.exec(each)
  return db
}

// A virtual table makes its own shadow tables, so replaying them too would collide.
function replay(entries: Entry[]) {
  const virtual = entries.filter(({ sql }) => /^create\s+virtual\s+table/i.test(sql)).map(({ name }) => name)
  const own = entries.filter(({ name, sql }) => sql && !INTERNAL.test(name) && !virtual.some((table) => name.startsWith(`${table}_`)))

  return own.sort((a, b) => ORDER.indexOf(a.type) - ORDER.indexOf(b.type)).map(({ sql }) => sql)
}

const described = (key: string, value: string) => (key.startsWith('check ') ? `${key.split(' ')[1]} ${value}` : `${key} (${value})`)

/* Production with its pending migrations on top against the schema a new database is born from. Each line names the side that is missing something, which is also the file that needs the edit. */
export function drift(production: Entry[], pending: { file: string; sql: string }[], schema: string) {
  const issues: string[] = []

  const prod = built(replay(production))
  for (const { file, sql } of pending) {
    try {
      prod.exec(sql)
    } catch (error) {
      return [`${file} fails on production, ${(error as Error).message}.`]
    }
  }

  let fresh: DatabaseSync
  try {
    fresh = built([schema])
  } catch (error) {
    return [`schema.sql does not build, ${(error as Error).message}.`]
  }

  const want = facts(fresh)
  const have = facts(prod)

  for (const [key, value] of want) {
    if (!have.has(key)) issues.push(`${described(key, value)} is in schema.sql, but neither production nor a pending migration makes it. Add a migration.`)
    else if (!key.startsWith('check ') && have.get(key) !== value) issues.push(`${key} is ${have.get(key)} on production with its migrations, but ${value} in schema.sql. Align one with the other.`)
  }

  for (const [key, value] of have) {
    if (!want.has(key)) issues.push(`${described(key, value)} is on production with its migrations, but missing from schema.sql. Add it there.`)
  }

  return issues
}

// A migration production already recorded has done its work, and the schema carries it from here.
export const stale = (files: string[], applied: string[]) => files.filter((file) => applied.includes(file)).map((file) => `${file} is already applied on production. Delete it from site/db/migrations.`)

async function main() {
  const strict = process.argv.includes('--strict')
  const say = (level: 'notice' | 'warning' | 'error', text: string) => console.log(process.env.GITHUB_ACTIONS ? `::${level}::${text}` : `${level}: ${text}`)

  const id = await find_database()
  if (!id) return say('notice', `D1 "${DB_NAME}" does not exist yet, the first ship builds it from schema.sql.`)

  const entries = await query<Entry>(id, 'select type, name, sql from sqlite_master where sql is not null')
  const ledger = entries.some(({ name }) => name === 'd1_migrations') ? await query<{ name: string }>(id, 'select name from d1_migrations') : []

  const files = migrations()
  const applied = ledger.map(({ name }) => name)
  const pending = files.filter((file) => !applied.includes(file)).map((file) => ({ file, sql: migration(file) }))

  const issues = [...stale(files, applied), ...drift(entries, pending, readFileSync(SCHEMA, 'utf8'))]
  if (issues.length === 0) return say('notice', `Production, ${pending.length} pending migration${pending.length === 1 ? '' : 's'} and schema.sql agree.`)

  for (const issue of issues) say(strict ? 'error' : 'warning', issue)
  if (strict) process.exit(1)
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main().catch((error) => {
    console.error(error)
    process.exit(1)
  })
}
