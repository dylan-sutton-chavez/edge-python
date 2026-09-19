export const PALETTES = ['sand', 'moss', 'sky'] as const
export const ICONS = 16

export type Palette = (typeof PALETTES)[number]
export type Avatar = { icon: number; palette: Palette }

export function avatarFor(seed: string): Avatar {
  let hash = 0x811c9dc5

  for (const char of seed) {
    hash ^= char.codePointAt(0)!
    hash = Math.imul(hash, 0x01000193) >>> 0
  }

  return { icon: (hash % ICONS) + 1, palette: PALETTES[(hash >>> 8) % PALETTES.length]! }
}

export function readAvatar(form: HTMLFormElement): Avatar {
  const value = (name: string) => form.querySelector<HTMLInputElement>(`input[name="${name}"]:checked`)!.value

  return { icon: Number(value('icon')), palette: value('palette') as Palette }
}

export function pickAvatar(form: HTMLFormElement, { icon, palette }: Avatar) {
  const radio = (name: string, value: string | number) => form.querySelector<HTMLInputElement>(`input[name="${name}"][value="${value}"]`)!

  radio('palette', palette).checked = true
  radio('icon', icon).checked = true

  form.querySelectorAll<HTMLElement>('[class*="avatar-"]').forEach((el) => {
    el.className = el.className.replace(/avatar-\w+/, `avatar-${palette}`)
  })

  form.querySelector('[data-preview] > span')!.innerHTML = radio('icon', icon).nextElementSibling!.innerHTML
}
