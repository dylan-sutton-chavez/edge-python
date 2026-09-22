delete from account where provider = 'email';

create unique index account_one_per_provider on account(user_id, provider);
