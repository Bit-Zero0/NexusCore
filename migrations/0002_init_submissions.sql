create table if not exists oj.submissions (
    submission_id varchar(128) primary key,
    problem_id varchar(128) not null references oj.problems(problem_id) on delete cascade,
    user_id varchar(128) not null,
    language varchar(32) not null,
    source_code text not null,
    status varchar(32) not null,
    created_at timestamptz not null default now()
);

create index if not exists idx_oj_submissions_problem_id on oj.submissions (problem_id);
create index if not exists idx_oj_submissions_user_id on oj.submissions (user_id);
