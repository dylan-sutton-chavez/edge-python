import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { sha256hex } from '../../lib/crypto'
import { json } from '../../lib/server/http'
import { tokenUser } from '../../lib/server/tokens'
import type { Page } from '../../lib/server/packages'
import { MAX_ARTIFACT, MAX_DESCRIPTION, MAX_NEW_NAMES, claimedToday, described, keyOf, named, packageByName, pages, publish, versionExists, versioned } from '../../lib/server/packages'

// The bundle is stored as it arrived. Its format lives in the CLI, so nothing here has to learn it.
export const POST: APIRoute = async ({ request }) => {
  const userId = await tokenUser(env.DB, request.headers.get('authorization'))
  if (!userId) return json({ error: 'That token is not valid.' }, 401)

  const form = await request.formData().catch(() => null)
  const artifact = form?.get('artifact')
  const manifest = form?.get('manifest')
  const carried = form?.get('docs')

  if (!(artifact instanceof File) || typeof manifest !== 'string') return json({ error: 'Send a manifest and an artifact.' }, 400)

  // A bundle with no docs directory sends nothing, and a hand-built request can send anything.
  let declared: { name?: string; version?: string; description?: string | null }
  let docs: Page[]

  try {
    declared = JSON.parse(manifest)
    docs = pages(typeof carried === 'string' ? JSON.parse(carried) : {})
  } catch (error) {
    const why = error instanceof SyntaxError ? 'Send the manifest and the doc pages as JSON.' : (error as Error).message
    return json({ error: why }, 400)
  }

  const { name, version, description } = declared

  if (typeof name !== 'string' || !named(name)) return json({ error: 'A name is lowercase letters, digits and single hyphens, starting with a letter.' }, 400)
  if (typeof version !== 'string' || !versioned(version)) return json({ error: 'A version is major.minor.patch, digits only.' }, 400)
  if (!described(description)) return json({ error: `A description is ${MAX_DESCRIPTION} characters at most.` }, 400)
  if (artifact.size > MAX_ARTIFACT) return json({ error: `An artifact is ${MAX_ARTIFACT} bytes at most.` }, 413)

  const held = await packageByName(env.DB, name)
  if (held && held.user_id !== userId) return json({ error: `The name ${name} belongs to someone else.` }, 409)

  // A name nobody holds is the scarce thing, so it costs more than another version of your own.
  const limit = held ? env.PUBLISH_VERSION : env.PUBLISH_NAME
  if (!(await limit.limit({ key: userId })).success) return json({ error: 'Too many packages published. Try again later.' }, 429)

  if (!held && (await claimedToday(env.DB, userId)) >= MAX_NEW_NAMES) {
    return json({ error: `You can claim ${MAX_NEW_NAMES} names a day.` }, 429)
  }

  if (await versionExists(env.DB, name, version)) return json({ error: `${name} ${version} is already published.` }, 409)

  const bytes = await artifact.arrayBuffer()
  const digest = await sha256hex(new Uint8Array(bytes))
  const key = keyOf(name, version)

  await env.CDN_BUCKET.put(key, bytes, { httpMetadata: { contentType: 'application/octet-stream' } })
  await publish(env.DB, userId, { name, version, digest, size: bytes.byteLength, description: description ?? null, docs })

  return json({ name, version, digest, url: `${env.CDN}/${key}` }, 201)
}
