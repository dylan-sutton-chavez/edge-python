import { env } from 'cloudflare:workers'
import { CODE_MINUTES, type Purpose } from '../otp'

/* What the code is for, in the subject and in the body, so nobody spends one on something they did not ask for. */
const FOR: Record<Purpose, { subject: string; what: string }> = {
  sign_in: { subject: 'is your sign-in code', what: 'sign in' },
  change_email: { subject: 'confirms your new address', what: 'send your codes to this address' },
  delete_account: { subject: 'confirms deleting your account', what: 'delete your account' }
}

/* Tells the address it is losing the account, so a move nobody asked for does not go unseen. */
export async function sendMovedMail(to: string, moved: string) {
  await env.EMAIL.send({
    from: { name: 'Edge Python', email: env.EMAIL_FROM },
    to,
    subject: 'Your Edge Python address changed',
    text: `Sign-in codes now go to ${moved}. This address no longer reaches the account.\n\nIf you didn't do this, reply to this email.`,
    html: `<p>Sign-in codes now go to ${moved}. This address no longer reaches the account.</p><p>If you didn't do this, reply to this email.</p>`
  })
}

export async function sendCodeMail(to: string, code: string, purpose: Purpose) {
  const { subject, what } = FOR[purpose]
  const line = `Your code is ${code}, and it expires in ${CODE_MINUTES} minutes. Use it to ${what}.`

  await env.EMAIL.send({
    from: { name: 'Edge Python', email: env.EMAIL_FROM },
    to,
    subject: `${code} ${subject}`,
    text: `${line}\n\nIf you didn't request it, ignore this email.`,
    html: `<p>${line}</p><p>If you didn't request it, ignore this email.</p>`
  })
}
