create table user (
  id text primary key,
  email text not null unique,
  name text,
  handle text unique,
  avatar_icon integer,
  avatar_palette text,
  created_at integer not null
);

create table account (
  provider text not null,
  provider_id text not null,
  user_id text not null references user(id) on delete cascade,
  primary key (provider, provider_id)
);

create table session (
  id text primary key,
  user_id text not null references user(id) on delete cascade,
  expires_at integer not null
);

create index session_user on session(user_id);

create table email_code (
  email text primary key,
  hash text not null,
  expires_at integer not null,
  attempts integer not null default 0
);
