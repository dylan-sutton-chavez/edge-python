# Runbook

## Production approval

Secrets are repository secrets, kept in one place. The pause before production is the `production` environment under Settings, Environments, with `dylan-sutton-chavez` as its required reviewer, so a `v` tag waits on Ship until the run is approved.

## What a deploy keeps

A push to `main` promotes to dev and a `v` tag ships to production, and either one replaces the files the last build shipped.

- Dev rebuilds its database from the schema and the seed, and sweeps the packages published under `pkg/` with it.
- Production only runs the migrations its database has not recorded, and keeps every published package.
- A frozen release, the copy a tag from `v1.0.0` keeps under its version, stays in both.

## The owner account

`OWNER` in `site/src/lib/account/handle.ts` names the account that publishes the standard library. It skips the per-minute publish limiter and gets five times the daily names, versions and room, and since the privilege follows the handle, that account keeps `@dylan`.

## Migrations

A schema change edits `site/db/schema.sql` and adds its step to `site/db/migrations/` in the same commit, since production keeps its rows. The next `v` tag runs the step before the Worker ships, and after that release you delete the file by hand, which the Database job warns about until you do.

`npm run schema` in `infra/` reads production and checks that it plus the pending migrations matches `schema.sql`. The Database job warns about a mismatch on `main` and enforces it on a tag.

## The Discord bot

`DISCORD_TOKEN` is the only secret the bot needs, and it has no environment of its own since it answers one server. It registers no commands and reads messages over the REST API, so the MESSAGE CONTENT intent is what the token depends on rather than anything in the code, and without it a mention of its role arrives with no text. Resetting the token in the developer portal invalidates the old one at once, so the secret changes in the same move.

A Durable Object alarm asks Discord what was said since the id in `cursor`, which is why nothing is held open and a restart only reads that row again. `bot.yml` ships it on a push that touches `bot/` and never on a tag, since the bot reads `/SKILL.md` and searches the site under `/api` over http the way any reader does. It answers a mention of it or of its role, or a reply to it, in any channel it was let into, and `GUILD` fixes the one server. `budget` keeps each day's spending beside a `quiet` switch that every new day inherits, so setting it to 1 on the newest row silences both doors until it is set back, with no deploy.
