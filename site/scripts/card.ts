import { readFileSync, writeFileSync } from 'node:fs'
import sharp from 'sharp'

// The card is authored once as vector, every preview wants the raster.
const SIZE = { width: 1200, height: 630 }

const source = new URL('../src/assets/og.svg', import.meta.url)
const target = new URL('../public/og.png', import.meta.url)

const png = await sharp(readFileSync(source)).resize(SIZE).png().toBuffer()
writeFileSync(target, png)

console.log(`Rendered the social card, ${SIZE.width}x${SIZE.height}, ${(png.length / 1024).toFixed(0)} KB.`)
