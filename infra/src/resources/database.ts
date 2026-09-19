import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { client, account_id } from '../client'
import { DB_NAME, SITE_DIR } from '../constants'

// Never deleted, only emptied. Dropping a D1 is irreversible and would change the binding id.
export async function ensure_database() {
  for await (const db of client.d1.database.list({ account_id, name: DB_NAME })) {
    if (db.name === DB_NAME) return db.uuid!
  }

  console.log(`Creating D1 "${DB_NAME}"...`)
  const db = await client.d1.database.create({ account_id, name: DB_NAME })
  return db.uuid!
}

export async function reset_database() {
  const id = await ensure_database()

  const query = async <T>(sql: string) => {
    for await (const result of client.d1.database.query(id, { account_id, sql })) return (result.results ?? []) as T[]
    return [] as T[]
  }

  const tables = await query<{ name: string }>("select name from sqlite_master where type = 'table' and name not like '\\_cf\\_%' escape '\\' and name not like 'sqlite\\_%' escape '\\'")
  for (const { name } of tables) await query(`drop table if exists "${name}"`)

  execFileSync('npm', ['run', 'config'], { cwd: SITE_DIR, stdio: 'inherit' })
  execFileSync('npx', ['wrangler', 'd1', 'migrations', 'apply', DB_NAME, '--remote'], { cwd: SITE_DIR, stdio: 'inherit' })

  await query(readFileSync(`${SITE_DIR}db/seed.sql`, 'utf8'))
  console.log(`D1 "${DB_NAME}" reset and seeded.`)
}
