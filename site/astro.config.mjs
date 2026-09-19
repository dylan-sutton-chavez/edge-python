import { readFile } from 'node:fs/promises'
import { basename } from 'node:path'
import { defineConfig, fontProviders } from 'astro/config'
import cloudflare from '@astrojs/cloudflare'
import mdx from '@astrojs/mdx'
import tailwindcss from '@tailwindcss/vite'
import { unified } from '@astrojs/markdown-remark'
import { remarkPlayground } from './src/lib/docs/remark-playground'

// The Cloudflare plugin ends Vite environments, dropping Astro's dev font map, so fonts come from .astro/fonts.
const devFonts = {
  name: 'dev-fonts',
  apply: 'serve',
  configureServer(server) {
    server.middlewares.use('/_astro/fonts', async (request, response, next) => {
      const id = basename(new URL(request.url ?? '/', 'http://localhost').pathname)

      try {
        const file = await readFile(new URL(`./.astro/fonts/${id}`, import.meta.url))
        response.setHeader('content-type', `font/${id.split('.').pop()}`)
        response.end(file)
      } catch {
        next()
      }
    })
  }
}

export default defineConfig({
  output: 'server',
  adapter: cloudflare({ imageService: 'passthrough' }),
  session: false,
  server: { port: 4322 },
  integrations: [mdx()],
  markdown: {
    shikiConfig: {
      themes: { light: 'github-light', dark: 'github-dark' },
      defaultColor: false,
      langAlias: { 'edge-python': 'python', output: 'text' }
    },
    processor: unified({ remarkPlugins: [remarkPlayground] })
  },
  fonts: [
    { provider: fontProviders.google(), name: 'Inter', cssVariable: '--font-inter', weights: ['100 900'], styles: ['normal'] },
    { provider: fontProviders.google(), name: 'JetBrains Mono', cssVariable: '--font-jetbrains-mono', weights: ['100 800'], styles: ['normal'], fallbacks: ['monospace'] }
  ],
  vite: {
    plugins: [tailwindcss(), devFonts],
    optimizeDeps: { include: ['astro/assets/services/noop'] },
    server: { watch: { usePolling: true, interval: 300, ignored: ['**/.wrangler/**'] } }
  }
})
