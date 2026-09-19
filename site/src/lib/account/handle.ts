export const RESERVED = ['404', 'api', 'docs', 'packages', 'publish', 'settings', 'terms', 'welcome']

export function validateHandle(value: string): string | null {
  if (value.length < 3) return 'At least 3 characters.'
  if (value.length > 20) return 'At most 20 characters.'
  if (!/^[a-z0-9-]+$/.test(value)) return 'Lowercase letters, numbers and hyphens only.'
  if (value.startsWith('-') || value.endsWith('-')) return "Can't start or end with a hyphen."
  if (RESERVED.includes(value)) return 'That name is reserved.'
  return null
}
