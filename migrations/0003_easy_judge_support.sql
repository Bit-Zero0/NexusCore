alter table oj.problems
    add column if not exists easy_config jsonb;

alter table oj.submissions
    add column if not exists score integer not null default 0,
    add column if not exists max_score integer not null default 0,
    add column if not exists result_message text;
