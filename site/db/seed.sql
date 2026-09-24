insert into user (id, handle, name, avatar_icon, avatar_palette, created_at, updated_at)
values ('u_unclaimed', 'unclaimed', 'Unclaimed', 1, 'sand', unixepoch() * 1000, unixepoch() * 1000);

insert into account (provider, provider_id, user_id, created_at)
values ('email', 'unclaimed@edgepython.com', 'u_unclaimed', unixepoch() * 1000);
