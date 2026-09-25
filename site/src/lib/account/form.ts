export function setError(input: HTMLInputElement, message: string | null) {
  const error = input.closest('div')?.querySelector<HTMLElement>('[data-error]')
  if (!error) return

  error.textContent = message ?? ''
  error.hidden = !message
  input.setAttribute('aria-invalid', String(message !== null))
}

// A touch keyboard covers the popup that just opened, so only a hover pointer gets the field.
export function offer(field?: HTMLElement | null) {
  if (matchMedia('(hover: hover)').matches) field?.focus({ preventScroll: true })
}

export async function pending<T>(button: HTMLButtonElement, task: () => Promise<T>) {
  button.disabled = true
  button.toggleAttribute('data-busy', true)

  try {
    return await task()
  } finally {
    button.disabled = false
    button.toggleAttribute('data-busy', false)
  }
}
