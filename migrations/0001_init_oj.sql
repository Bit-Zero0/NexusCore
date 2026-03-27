create schema if not exists oj;

create table if not exists oj.problems (
    problem_id varchar(128) primary key,
    title varchar(255) not null,
    slug varchar(255) not null unique,
    judge_mode varchar(32) not null,
    statement_md text not null default '',
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table if not exists oj.problem_limits (
    problem_id varchar(128) not null references oj.problems(problem_id) on delete cascade,
    language varchar(32) not null,
    time_limit_ms bigint not null,
    memory_limit_kb bigint not null,
    primary key (problem_id, language)
);

create index if not exists idx_oj_problems_judge_mode on oj.problems (judge_mode);
