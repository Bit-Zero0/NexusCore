alter table oj.problems
    add column if not exists sandbox_kind varchar(32) not null default 'nsjail';

create index if not exists idx_oj_problems_sandbox_kind
    on oj.problems (sandbox_kind);
