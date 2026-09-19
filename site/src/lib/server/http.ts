import type { APIContext } from 'astro'

export const json = (data: unknown, status = 200) => Response.json(data, { status })

export async function body<T extends Record<string, unknown>>(request: Request): Promise<Partial<T>> {
  try {
    return (await request.json()) as Partial<T>
  } catch {
    return {}
  }
}

export const ip = (context: APIContext) => context.request.headers.get('cf-connecting-ip') ?? context.clientAddress
