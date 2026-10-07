const REGISTRY = 'https://registry.npmjs.org/lucide-static/latest'

async function json(url: string) {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`Fetching ${url} answered ${response.status}.`)
  return response.json()
}

// A release named on the command line, or the latest one Lucide published, so every build is fresh.
const version: string = Deno.args[0] || (await json(REGISTRY)).version
const tags: Record<string, string[]> = await json(`https://cdn.jsdelivr.net/npm/lucide-static@${version}/tags.json`)
const icons = Object.keys(tags).sort()
Deno.mkdirSync('target', { recursive: true })
Deno.writeTextFileSync('target/lucide.json', JSON.stringify({ version, icons }))
console.log(`Wrote ${icons.length} icons of Lucide ${version}.`)
