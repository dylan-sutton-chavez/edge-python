import { defineCollection, z } from 'astro:content'
import { glob } from 'astro/loaders'

export const collections = {
  docs: defineCollection({
    loader: glob({ base: '../docs', pattern: '**/*.mdx', generateId: ({ entry }) => entry.replace(/\.mdx$/, '') }),
    schema: z.object({ title: z.string().optional(), description: z.string().optional() })
  })
}
