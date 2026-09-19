declare namespace App {
  interface Locals {
    user: import('./lib/account/auth').Me | null
  }
}

declare namespace Cloudflare {
  interface Env {
    OAUTH_GITHUB_ID: string
    OAUTH_GITHUB_SECRET: string
    OAUTH_GOOGLE_ID: string
    OAUTH_GOOGLE_SECRET: string
  }
}
