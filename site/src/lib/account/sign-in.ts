import { sendCode, verifyCode, checkHandle, saveProfile } from './auth'
import { validateHandle } from './handle'
import { avatarFor, pickAvatar, readAvatar } from './avatar'
import { setError, pending, offer } from './form'
import { CODE_LIFE } from '../otp'

export type Step = 'providers' | 'code' | 'user' | 'avatar'

type Screen = { title: string; text: string; next?: Step; submit?: () => Promise<boolean | 'done'> }

const PROFILE: Step[] = ['user', 'avatar']
const KEY = 'signin-pending'

type Pending = { step: Step; email?: string; until?: number }

export function createSignIn(dialog: HTMLDialogElement) {
  const find = <T extends Element>(selector: string) => dialog.querySelector<T>(selector)!
  const form = (step: Step) => find<HTMLFormElement>(`form[data-panel="${step}"]`)
  const input = (name: string) => find<HTMLInputElement>(`input[name="${name}"]:not([type="radio"])`)
  const track = find<HTMLElement>('[data-panels]')
  const next = find<HTMLButtonElement>('[data-next]')
  const panels = dialog.querySelectorAll<HTMLElement>('[data-panel]')

  let email = ''
  let until = 0

  const STEPS: Record<Step, Screen> = {
    providers: { title: 'Sign In', text: 'Sign in to publish packages and pin them by sha256.', next: 'code', submit: start },
    code: { title: 'Check your email', text: 'We sent a 6-digit code to {email}.', next: 'user', submit: verify },
    user: { title: 'Create your profile', text: 'Pick a handle. It becomes your profile URL.', next: 'avatar', submit: claim },
    avatar: { title: 'Create your profile', text: 'Choose how you show up.', submit: save }
  }

  async function start() {
    const field = input('email')
    email = field.value.trim().toLowerCase()

    try {
      await sendCode('sign_in', email)
    } catch (error) {
      setError(field, (error as Error).message)
      return false
    }

    until = Date.now() + CODE_LIFE
    setError(field, null)
    form('code').reset()
    setError(input('code'), null)
    return true
  }

  async function verify() {
    const code = input('code')
    const { ok, handle } = await verifyCode(email, code.value.trim()).catch(() => ({ ok: false, handle: null }))
    setError(code, ok ? null : "That code didn't work. Check it and try again.")
    if (!ok) return false
    if (handle) return 'done'

    pickAvatar(form('avatar'), avatarFor(email))
    return true
  }

  async function claim() {
    const handle = input('handle')
    const problem = validateHandle(handle.value) ?? ((await checkHandle(handle.value)) ? null : `@${handle.value} is already taken.`)
    setError(handle, problem)
    return !problem
  }

  async function save() {
    try {
      await saveProfile({ handle: input('handle').value, name: input('profile-name').value.trim(), avatar: readAvatar(form('avatar')) })
    } catch (error) {
      setError(input('handle'), (error as Error).message)
      go('user')
      return false
    }

    return true
  }

  const current = () => dialog.dataset.step as Step
  const active = () => find<HTMLElement>(`[data-panel="${current()}"]`)
  const fit = () => { track.style.height = `${active().offsetHeight}px` }

  const remember = (held: Pending | null) => {
    try {
      held ? sessionStorage.setItem(KEY, JSON.stringify(held)) : sessionStorage.removeItem(KEY)
    } catch {}
  }

  /* A record whose code already expired is not worth returning to, so it comes back empty. */
  const recall = (): Pending | null => {
    try {
      const held = JSON.parse(sessionStorage.getItem(KEY) ?? 'null') as Pending | null
      return held && (!held.until || held.until > Date.now()) ? held : null
    } catch {
      return null
    }
  }

  function go(step: Step) {
    dialog.dataset.step = step
    panels.forEach((panel) => panel.toggleAttribute('data-active', panel.dataset.panel === step))
    fit()

    find('[data-title]').textContent = STEPS[step].title
    find('[data-subtitle]').textContent = STEPS[step].text.replace('{email}', email)

    const index = PROFILE.indexOf(step)
    find<HTMLElement>('[data-footer]').hidden = index === -1
    find<HTMLElement>('[data-back]').hidden = index <= 0
    next.querySelector('[data-label]')!.textContent = STEPS[step].next ? 'Next' : 'Finish'
    next.setAttribute('form', `signin-${step}`)
    dialog.querySelectorAll('[data-dots] li').forEach((dot, i) => (i === index ? dot.setAttribute('aria-current', 'step') : dot.removeAttribute('aria-current')))
    remember(step === 'code' ? { step, email, until } : index === -1 ? null : { step })

    offer(active().querySelector<HTMLElement>('input:not([type="radio"])'))
  }

  function open(step?: Step) {
    const stored = recall()

    // A code already in an inbox stays good, so returning lands on it.
    if (stored?.email) {
      email = stored.email
      until = stored.until ?? 0
    }

    const resumable = stored && (stored.step === 'code' || PROFILE.includes(stored.step))

    dialog.showModal()
    track.style.transition = 'none'
    go(step ?? (resumable ? stored!.step : 'providers'))
    void track.offsetHeight
    track.style.transition = ''
  }

  function finish() {
    dialog.close()
    dialog.querySelectorAll('form').forEach((each) => each.reset())
    go('providers')
    dialog.dispatchEvent(new CustomEvent('signin:done', { bubbles: true }))
  }

  dialog.addEventListener('submit', async (event) => {
    event.preventDefault()
    const target = event.target as HTMLFormElement
    const screen = STEPS[target.closest<HTMLElement>('[data-panel]')!.dataset.panel as Step]
    const button = target.querySelector<HTMLButtonElement>('button:not([type="button"])') ?? next
    const ok = screen.submit ? await pending(button, screen.submit) : true
    if (ok === 'done') finish()
    else if (ok) screen.next ? go(screen.next) : finish()
  })

  const handle = input('handle')
  handle.addEventListener('input', () => {
    handle.value = handle.value.toLowerCase()
    setError(handle, handle.value ? validateHandle(handle.value) : null)
  })

  form('avatar').addEventListener('change', () => pickAvatar(form('avatar'), readAvatar(form('avatar'))))

  find('[data-resend]').addEventListener('click', async () => {
    if (!email) return go('providers')

    try {
      await sendCode('sign_in', email, true)
    } catch (error) {
      setError(input('code'), (error as Error).message)
      return
    }

    until = Date.now() + CODE_LIFE
    remember({ step: 'code', email, until })
    form('code').reset()
    setError(input('code'), null)
    find('[data-subtitle]').textContent = `We sent a new code to ${email}.`
  })

  const providers = dialog.querySelectorAll('a[href^="/api/auth/"]')
  providers.forEach((provider) => provider.addEventListener('click', () => provider.toggleAttribute('data-busy', true)))
  addEventListener('pageshow', () => providers.forEach((provider) => provider.toggleAttribute('data-busy', false)))

  find('[data-restart]').addEventListener('click', () => go('providers'))
  find('[data-back]').addEventListener('click', () => go(PROFILE[PROFILE.indexOf(current()) - 1]!))

  const observer = new ResizeObserver(() => { if (dialog.open) fit() })
  panels.forEach((panel) => observer.observe(panel))

  return { open }
}
