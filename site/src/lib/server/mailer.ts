import { env } from 'cloudflare:workers'

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

export async function sendCodeMail(to: string, code: string) {
  await env.EMAIL.send({
    from: { name: 'Edge Python', email: env.EMAIL_FROM },
    to,
    subject: `${code} is your Edge Python code`,
    text: `Your code is ${code}. It expires in 10 minutes.\n\nIf you didn't request it, ignore this email.`,
    html: `<p>Your code is ${code}. It expires in 10 minutes.</p><p>If you didn't request it, ignore this email.</p>`
  })
}
