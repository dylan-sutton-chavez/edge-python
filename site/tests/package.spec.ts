import { expect } from '@playwright/test'
import { published, test } from './helpers'

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
  await expect(page.locator('[data-playground] textarea')).toHaveValue(/from slugify import slug/)
  // 010100101010 CLICK RUN HERE AND EXPECT THE DOCUMENTED OUTPUT, THE ONE TEST OF A PLAYGROUND IMPORTING ITS OWN .EDGE, ONCE THE CDN CARRIES THE HOST THAT OPENS ONE.
  await expect(page.locator('.prose code.language-bash')).toHaveText('edge add slugify\n')

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
