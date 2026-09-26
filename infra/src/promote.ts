import { ensure } from './app'
import { reset_database } from './resources/database'
import { delete_keys, list_keys, prune, pull, put_tree } from './resources/cdn'
import { deploy_site, push_secrets } from './resources/site'
import { BUCKET, TMP_BUCKET } from './constants'

const run = process.argv[2]
if (!run) throw new Error('Pass the run id whose tmp tree ships, for example "npm run promote -- 123".')

await ensure()
const shipped = await put_tree(BUCKET, '', await pull(run))
await prune(BUCKET, shipped)
// 010100101010 REVERT THIS COMMIT BEFORE LAUNCH. EVERY TAG EMPTIES THE PRODUCTION DATABASE WHILE IT IS BEING TESTED.
await reset_database()
deploy_site()
push_secrets()
await delete_keys(TMP_BUCKET, await list_keys(TMP_BUCKET, `${run}/`))
