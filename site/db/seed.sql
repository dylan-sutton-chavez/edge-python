insert into user (id, handle, name, avatar_icon, avatar_palette, created_at, updated_at)
values ('u_dylan', 'dylan', 'Dylan', 3, 'moss', unixepoch() * 1000, unixepoch() * 1000);

insert into account (provider, provider_id, user_id, created_at)
values ('email', 'c.sutton.dylan@gmail.com', 'u_dylan', unixepoch() * 1000);
