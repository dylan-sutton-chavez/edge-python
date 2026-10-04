-- Where each channel was last read, so a restart resumes instead of answering what already passed.
create table if not exists cursor (
  channel text primary key,
  last_id text not null
) strict;

-- Keyed by a digest so the http side keeps only the hash of the secret it hands out, and the kind keeps each door's ids apart.
create table if not exists session (
  id text primary key,
  kind text not null check (kind in ('discord', 'http')),
  turns text not null default '[]',
  born_at integer not null,
  last_at integer not null
) strict;

create index if not exists session_last on session(last_at);

-- Every message the bot sent and the thread it belongs to, so a reply finds its own without walking the chain back.
create table if not exists thread (
  message_id text primary key,
  session text not null references session(id) on delete cascade
) strict;

-- One row a day, and each new day opens with the quiet of the last so a silenced bot stays silent until set back.
create table if not exists budget (
  day text primary key,
  spent integer not null default 0,
  quiet integer not null default 0
) strict;
