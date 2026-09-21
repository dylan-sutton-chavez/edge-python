create table token (
  id text primary key,
  user_id text not null references user(id) on delete cascade,
  name text not null,
  salt text not null,
  hash text not null,
  created_at integer not null,
  used_at integer,
  expires_at integer
);

create index token_user on token(user_id);
