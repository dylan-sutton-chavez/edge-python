import { resolve } from 'node:path'
import { ensure_tmp_cdn, put_tree } from './resources/cdn'
import { TMP_BUCKET, TMP_CDN_URL } from './constants'

const [run, tree] = process.argv.slice(2)
if (!run || !tree) throw new Error('Pass the run id and a staged tree, for example "npm run upload -- 123 ../_cdn".')

await ensure_tmp_cdn()
const keys = await put_tree(TMP_BUCKET, `${run}/`, resolve(tree))
console.log(`Uploaded ${keys.length} object(s) to ${TMP_CDN_URL}/${run}/.`)
