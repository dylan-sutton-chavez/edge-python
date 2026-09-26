# Runbook

## Production approval

Secrets are repository secrets, kept in one place. The pause before production is the `production` environment under Settings, Environments, with `dylan-sutton-chavez` as its required reviewer, so a `v` tag waits on Ship until the run is approved.

## Migrations

A schema change edits `site/db/schema.sql` and adds its step to `site/db/migrations/` in the same commit. Production runs the step on the next `v` tag, and after that release you delete the file by hand, which the Database job warns about until you do.
