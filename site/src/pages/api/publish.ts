import type { APIRoute } from 'astro'
import { env } from 'cloudflare:workers'
import { sha256hex } from '../../lib/crypto'
import { json } from '../../lib/server/http'
import { tokenUser } from '../../lib/server/tokens'
import type { Packed } from '../../lib/server/bundle'
import { packed } from '../../lib/server/bundle'
import { MAX_ARTIFACT, MAX_DESCRIPTION, MAX_NEW_NAMES, MAX_NOTICE, checkPages, claimedToday, described, keyOf, linked, named, noticed, packageByName, publish, versionExists, versioned } from '../../lib/server/packages'

/* The artifact is the only thing sent. Everything a listing shows is read out of it here, so a publisher declares nothing twice and cannot declare it differently from what they shipped. */
export const POST: APIRoute = async ({ request }) => {
  const userId = await tokenUser(env.DB, request.headers.get('authorization'))
  if (!userId) return json({ error: 'That token is not valid.' }, 401)

  const form = await request.formData().catch(() => null)
  const artifact = form?.get('artifact')

  if (!(artifact instanceof File)) return json({ error: 'Send an artifact.' }, 400)
  if (artifact.size > MAX_ARTIFACT) return json({ error: `An artifact is ${MAX_ARTIFACT} bytes at most.` }, 413)

  const bytes = await artifact.arrayBuffer()

  // A token holder can hand-build a bundle, so the archive and its pages are held to the same rules the CLI packs under.
  let declared: Packed

  try {
    declared = packed(new Uint8Array(bytes))
    checkPages(declared.docs)
  } catch (error) {
    return json({ error: (error as Error).message }, 400)
  }

  const { name, version, description, repository, notice } = declared

  if (typeof name !== 'string' || !named(name)) return json({ error: 'A name is lowercase letters, digits and single hyphens, starting with a letter.' }, 400)
  if (typeof version !== 'string' || !versioned(version)) return json({ error: 'A version is major.minor.patch, digits only.' }, 400)
  if (!described(description)) return json({ error: `A description is ${MAX_DESCRIPTION} characters at most.` }, 400)
  if (!linked(repository)) return json({ error: 'A repository is an https url a listing can link.' }, 400)
  if (!noticed(notice)) return json({ error: `A license notice is ${MAX_NOTICE} bytes at most.` }, 400)

  const held = await packageByName(env.DB, name)
  if (held && held.user_id !== userId) return json({ error: `The name ${name} belongs to someone else.` }, 409)

  // A name nobody holds is the scarce thing, so it costs more than another version of your own.
  const limit = held ? env.PUBLISH_VERSION : env.PUBLISH_NAME
  if (!(await limit.limit({ key: userId })).success) return json({ error: 'Too many packages published. Try again later.' }, 429)

  if (!held && (await claimedToday(env.DB, userId)) >= MAX_NEW_NAMES) {
    return json({ error: `You can claim ${MAX_NEW_NAMES} names a day.` }, 429)
  }

  if (await versionExists(env.DB, name, version)) return json({ error: `${name} ${version} is already published.` }, 409)

  const digest = await sha256hex(new Uint8Array(bytes))
  const key = keyOf(name, version)

  // Stored exactly as it arrived, so the digest a manifest pins is the digest of the bytes that ran.
  await env.CDN_BUCKET.put(key, bytes, { httpMetadata: { contentType: 'application/octet-stream' } })
  await publish(env.DB, userId, {
    name,
    version,
    digest,
    size: bytes.byteLength,
    description: typeof description === 'string' ? description : null
  })

  return json({ name, version, digest, url: `${env.CDN}/${key}` }, 201)
}
