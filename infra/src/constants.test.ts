import { strict as assert } from 'node:assert'
import { test } from 'node:test'
import { names } from './constants'

test('prod sits on the apex and every other environment under its own subdomain', () => {
  assert.equal(names('prod').site, 'edgepython.com')
  assert.equal(names('prod').cdn, 'cdn.edgepython.com')
  assert.equal(names('dev').site, 'dev.edgepython.com')
  assert.equal(names('dev').cdn, 'cdn.dev.edgepython.com')
})

test('dev and prod share no name at all', () => {
  const dev = names('dev')
  const prod = names('prod')

  for (const key of Object.keys(dev) as (keyof typeof dev)[]) {
    assert.notEqual(dev[key], prod[key], `dev and prod would both use "${dev[key]}" as ${key}`)
  }
})
