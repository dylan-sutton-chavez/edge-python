import { env } from 'cloudflare:workers'

export async function sendCodeMail(to: string, code: string) {
  await env.EMAIL.send({
    from: { name: 'Edge Python', email: env.EMAIL_FROM },
    to,
    subject: `${code} is your Edge Python code`,
    text: `Your code is ${code}. It expires in 10 minutes.\n\nIf you didn't request it, ignore this email.`,
    html: `<p>Your code is ${code}. It expires in 10 minutes.</p><p>If you didn't request it, ignore this email.</p>`
  })
}
