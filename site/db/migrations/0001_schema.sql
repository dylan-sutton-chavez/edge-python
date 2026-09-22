create table user (
  id text primary key,
  handle text unique,
  name text,
  bio text,
  avatar_icon integer,
  avatar_palette text,
  created_at integer not null,
  updated_at integer not null,
  handle_changed_at integer,
  check ((avatar_icon is null) = (avatar_palette is null))
) strict;

create table account (
  provider text not null check (provider in ('email', 'github', 'google')),
  provider_id text not null check (provider_id = lower(provider_id)),
  user_id text not null references user(id) on delete cascade,
  created_at integer not null,
  primary key (provider, provider_id)
) strict;

create unique index account_one_per_provider on account(user_id, provider);

create table session (
  token_hash text primary key,
  user_id text not null references user(id) on delete cascade,
  created_at integer not null,
  expires_at integer not null
) strict;

create index session_user on session(user_id);
create index session_expiry on session(expires_at);

create table email_code (
  email text primary key check (email = lower(email)),
  hash text not null,
  attempts integer not null default 0,
  created_at integer not null,
  expires_at integer not null
) strict;

create index email_code_expiry on email_code(expires_at);

create table token (
  id text primary key,
  user_id text not null references user(id) on delete cascade,
  name text not null check (length(name) > 0),
  salt text not null,
  hash text not null,
  created_at integer not null,
  used_at integer,
  expires_at integer
) strict;

create index token_user on token(user_id);
create index token_expiry on token(expires_at);

create table package (
  name text primary key check (name = lower(name)),
  user_id text references user(id) on delete set null,
  created_at integer not null
) strict;

create index package_user on package(user_id);

create table version (
  package text not null references package(name),
  version text not null,
  digest text not null,
  size integer not null,
  published_at integer not null,
  yanked_at integer,
  primary key (package, version)
) strict;

create index version_package on version(package);
