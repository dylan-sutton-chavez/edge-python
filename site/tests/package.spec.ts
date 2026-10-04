import { expect } from '@playwright/test'
import { mintToken, packed, published, signedIn, test } from './helpers'
import { OWNER } from '../src/lib/account/handle'

const INTRO = `---
title: Introduction
description: Where to start.
---

# Introduction

Turns text into a slug, with a \`normalise\` helper.

\`\`\`edge-python
from slugify import slug
print(slug('Hello World'))
\`\`\`

\`\`\`output
hello-world
\`\`\`

Then a list.

- one
- two

\`\`\`bash
edge add slugify
\`\`\`
`

const INSTALL = `---
title: Installation
description: How to add it.
---

# Installation

Declare it, then import it.
`

const DOCS = {
  '@docs/01-getting-started/01-introduction.mdx': INTRO,
  '@docs/01-getting-started/02-installation.mdx': INSTALL,
  LICENSE: 'Apache License\nVersion 2.0, January 2004'
}

/* One flow, because a page is only right if the row, the artifact and the markdown all reach it together. */
test('renders a published package from its row and its artifact', async ({ page, request }) => {
  const { name } = await published(request, DOCS)

  await page.goto(`/package/${name}`)

  // What the listing indexed, beside what was read back out of the bundle.
  await expect(page.locator('h1')).toHaveText(name)
  await expect(page.getByText('Turn text into a slug.')).toBeVisible()
  await expect(page.getByText('Apache-2.0')).toBeVisible()
  await expect(page.getByRole('link', { name: 'Repository' })).toHaveAttribute('href', 'https://github.com/x/slugify')

  // The markdown the worker rendered, prose and a runnable pair alike.
  await expect(page.locator('.prose li')).toHaveText(['one', 'two'])
  await expect(page.locator('[data-playground]')).toHaveCount(1)
  await expect(page.locator('[data-playground] [data-input]')).toHaveValue(/from slugify import slug/)
  // The example's edge.json declares the release as a project would, the lock stays out of it.
  await expect(page.locator('[data-playground] [data-manifest] textarea')).toHaveValue(new RegExp(`"${name}": "0\\.1\\.0"`))
  await expect(page.locator('.prose code.language-bash')).toHaveText('edge add slugify\n')

  // The bare address shows the first page, so it hands that page the one address the two of them share.
  await expect(page.locator('link[rel=canonical]')).toHaveAttribute('href', new RegExp(`/package/${name}/getting-started/introduction$`))

  // The aside orders both pages and every link stays inside the package.
  const links = page.locator('aside[data-sticky] a')
  await expect(links).toHaveText(['Introduction', 'Installation'])
  for (const href of await links.evaluateAll((all) => all.map((a) => a.getAttribute('href')))) {
    expect(href).toMatch(new RegExp(`^/package/${name}/`))
  }

  await expect(page.locator('table tbody tr')).toHaveCount(1)
})

test('opens a page the aside names and refuses one it does not', async ({ page, request }) => {
  const { name } = await published(request, DOCS)

  await page.goto(`/package/${name}/getting-started/installation`)
  await expect(page.locator('.prose h2')).toHaveText('Installation')

  await page.goto(`/package/${name}/nope`)
  await expect(page.getByText('Not found')).toBeVisible()
})

// An address is cheap to farm and a linked provider is not, so an account with only an address gets the smaller room.
test('shows an email account the room it gets and how to ask for more', async ({ page, context }) => {
  await signedIn(context)

  await page.goto('/settings')
  await page.locator('[data-panel="Storage"]').first().click()

  const panel = page.locator('[data-view="Storage"]')
  await expect(panel).toContainText('of 50.0 MB')
  await expect(panel).toContainText('Linking GitHub or Google raises this to 1.0 GB')
  await expect(panel).toContainText('Nothing published yet.')
  await expect(panel.getByRole('link', { name: 'Ask for more room' })).toHaveAttribute('href', /^mailto:.*subject=/)
})

test('lists every page of a documented package and its author in the sitemap, and leaves an undocumented one out', async ({ request }) => {
  const documented = await published(request, DOCS)
  const bare = await published(request)
  const sitemap = await (await request.get('/sitemap.xml')).text()

  expect(sitemap).toContain(`/package/${documented.name}/getting-started/introduction</loc>`)
  expect(sitemap).toContain(`/package/${documented.name}/getting-started/installation</loc>`)
  expect(sitemap).toContain(`/@${documented.handle}</loc>`)

  // The bare address points at the first page, so sending both would be sending the same page twice.
  expect(sitemap).not.toContain(`/package/${documented.name}</loc>`)

  expect(sitemap).not.toContain(`/package/${bare.name}`)
  expect(sitemap).not.toContain(`/@${bare.handle}</loc>`)
})

test('opens an older version under ?v= and keeps the newest at the bare address', async ({ page, request }) => {
  const { name } = await published(request, DOCS)
  const headers = { authorization: `Bearer ${await mintToken(request)}`, 'content-type': 'application/octet-stream' }
  const docs = { ...DOCS, '@docs/01-getting-started/01-introduction.mdx': INTRO.replace('Turns text into a slug', 'Turns text into a slug, now in 0.2.0') }
  const newer = await request.post('/api/publish', { headers, data: packed({ 'edge.json': JSON.stringify({ name, version: '0.2.0', edge: '0.7.0' }), 'main.py': '', ...docs }) })
  expect(newer.status()).toBe(201)

  await page.goto(`/package/${name}`)
  await expect(page.locator('h1 + span')).toHaveText('0.2.0')
  await expect(page.getByText('needs edge 0.7.0')).toBeVisible()
  await expect(page.locator('.prose')).toContainText('now in 0.2.0')
  await expect(page.locator('[data-code] pre').first()).toHaveText(`edge add ${name} && edge lock`)
  await expect(page.locator('table a')).toHaveText(['0.2.0', '0.1.0'])

  await page.locator('table a', { hasText: '0.1.0' }).click()
  await expect(page).toHaveURL(`/package/${name}?v=0.1.0`)
  await expect(page.locator('h1 + span')).toHaveText('0.1.0')
  await expect(page.getByText('needs edge')).toHaveCount(0)
  await expect(page.locator('.prose')).not.toContainText('now in 0.2.0')
  await expect(page.locator('[data-code] pre').first()).toHaveText(`edge add ${name}@0.1.0 && edge lock`)
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute('content', 'noindex, follow')
  await expect(page.locator('aside[data-sticky] a').first()).toHaveAttribute('href', `/package/${name}/getting-started/introduction?v=0.1.0`)

  await page.goto(`/package/${name}?v=0.2.0`)
  await expect(page).toHaveURL(`/package/${name}`)

  await page.goto(`/package/${name}?v=9.9.9`)
  await expect(page.getByText('Not found')).toBeVisible()
})

test('shelves programs beside the packages under Community and sends std to its maintainer', async ({ page, request }) => {
  const { name } = await published(request)

  await page.goto('/')
  await expect(page.locator('[data-cards]').getByRole('link', { name: 'Edge and rails' })).toHaveAttribute('href', '/program/edge-and-rails')
  await expect(page.locator(`[data-cards] a[href="/package/${name}"]`)).toHaveCount(1)

  await page.locator('[data-shelf] input').fill('nothing like it')
  await expect(page.locator('[data-cards]')).toHaveText('Nothing matches that.')

  await expect(page.locator('[data-sorts]').getByRole('link', { name: 'std', exact: true })).toHaveAttribute('href', `/@${OWNER}`)
})

test('says nothing is there for a name nobody published', async ({ page }) => {
  await page.goto('/package/nobody-here')
  await expect(page.getByText('Not found')).toBeVisible()
})

test.describe('the versions table on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } })

  // Three columns never fit, so each keeps its width and the table scrolls rather than squeezing the dates.
  test('scrolls sideways instead of cramming the columns', async ({ page, request }) => {
    const { name } = await published(request)

    await page.goto(`/package/${name}`)

    const room = await page.locator('table').evaluate((el) => ({
      table: el.getBoundingClientRect().width,
      client: el.parentElement!.clientWidth,
      scroll: el.parentElement!.scrollWidth
    }))

    expect(room.table).toBeGreaterThan(room.client)
    expect(room.scroll).toBeGreaterThan(room.client)
  })
})
