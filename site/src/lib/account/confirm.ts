type Ask = { title: string; body: string; action: string }

const CODE = 6
const AGAIN = 'Resend code'
const DONE = 'Code sent'

// Long enough to read, short enough that the button is back before anyone reaches for it again.
const HELD = 2500

const parts = () => {
  const dialog = document.querySelector<HTMLDialogElement>('[data-dialog="confirm"]')

  return {
    dialog,
    go: dialog?.querySelector<HTMLButtonElement>('[data-confirm-go]'),
    title: dialog?.querySelector<HTMLElement>('[data-title]'),
    body: dialog?.querySelector<HTMLElement>('[data-confirm-body]'),
    field: dialog?.querySelector<HTMLLabelElement>('[data-confirm-code]'),
    input: dialog?.querySelector<HTMLInputElement>('[name="confirm-code"]'),
    error: dialog?.querySelector<HTMLElement>('[data-confirm-error]'),
    hint: dialog?.querySelector<HTMLElement>('[data-confirm-resend]'),
    again: dialog?.querySelector<HTMLButtonElement>('[data-confirm-again]')
  }
}

/* Opens the dialog and settles when it closes, so Escape, Cancel and the backdrop all read as no. Resolves the typed code when one was asked for, or the empty string when it was not. */
function ask({ title, body, action }: Ask, wantsCode: boolean, resend?: () => Promise<unknown>): Promise<string | null> {
  const { dialog, go, title: heading, body: text, field, input, error, hint, again } = parts()
  if (!dialog || !go || !heading || !text || !field || !input || !error || !hint || !again) return Promise.resolve(null)

  heading.textContent = title
  text.textContent = body
  go.textContent = action
  field.hidden = !wantsCode
  hint.hidden = !wantsCode || !resend
  again.textContent = AGAIN
  again.disabled = false
  error.hidden = true
  input.value = ''

  return new Promise((resolve) => {
    let answer: string | null = null
    let held: ReturnType<typeof setTimeout> | undefined

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

    /* Asks for another code and clears the field, so the one already typed cannot be sent against the new hash. */
    const mail = async () => {
      clearTimeout(held)
      again.disabled = true
      again.textContent = AGAIN
      error.hidden = true

      try {
        await resend!()
        input.value = ''

        // The button says it landed and then offers itself again, so nothing else has to hold the news.
        again.textContent = DONE
        held = setTimeout(() => {
          again.textContent = AGAIN
          again.disabled = false
        }, HELD)
      } catch (problem) {
        error.textContent = (problem as Error).message
        error.hidden = false
        again.disabled = false
      }

      input.focus()
    }

    const done = () => {
      clearTimeout(held)
      go.removeEventListener('click', yes)
      again.removeEventListener('click', mail)
      resolve(answer)
    }

    go.addEventListener('click', yes)
    if (resend) again.addEventListener('click', mail)
    dialog.addEventListener('close', done, { once: true })
    dialog.showModal()

    if (wantsCode) input.focus()
  })
}

/* True only when the action button was pressed. */
export const confirmed = (question: Ask) => ask(question, false).then((answer) => answer !== null)

/* The code the visitor typed, or null when they backed out. A resend is offered only when the caller can mail another. */
export const confirmedWithCode = (question: Ask, resend?: () => Promise<unknown>) => ask(question, true, resend)
