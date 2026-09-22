type Ask = { title: string; body: string; action: string }

const CODE = 6

const parts = () => {
  const dialog = document.querySelector<HTMLDialogElement>('[data-dialog="confirm"]')

  return {
    dialog,
    go: dialog?.querySelector<HTMLButtonElement>('[data-confirm-go]'),
    title: dialog?.querySelector<HTMLElement>('[data-title]'),
    body: dialog?.querySelector<HTMLElement>('[data-confirm-body]'),
    field: dialog?.querySelector<HTMLLabelElement>('[data-confirm-code]'),
    input: dialog?.querySelector<HTMLInputElement>('[name="confirm-code"]'),
    error: dialog?.querySelector<HTMLElement>('[data-confirm-error]')
  }
}

/* Opens the dialog and settles when it closes, so Escape, Cancel and the backdrop all read as no. Resolves the typed code when one was asked for, or the empty string when it was not. */
function ask({ title, body, action }: Ask, wantsCode: boolean): Promise<string | null> {
  const { dialog, go, title: heading, body: text, field, input, error } = parts()
  if (!dialog || !go || !heading || !text || !field || !input || !error) return Promise.resolve(null)

  heading.textContent = title
  text.textContent = body
  go.textContent = action
  field.hidden = !wantsCode
  error.hidden = true
  input.value = ''

  return new Promise((resolve) => {
    let answer: string | null = null

    const yes = () => {
      const code = input.value.trim()

      if (wantsCode && code.length !== CODE) {
        error.textContent = `The code is ${CODE} digits.`
        error.hidden = false
        input.focus()
        return
      }

      answer = code
      dialog.close()
    }

    const done = () => {
      go.removeEventListener('click', yes)
      resolve(answer)
    }

    go.addEventListener('click', yes)
    dialog.addEventListener('close', done, { once: true })
    dialog.showModal()

    if (wantsCode) input.focus()
  })
}

/* True only when the action button was pressed. */
export const confirmed = (question: Ask) => ask(question, false).then((answer) => answer !== null)

/* The code the visitor typed, or null when they backed out. */
export const confirmedWithCode = (question: Ask) => ask(question, true)
