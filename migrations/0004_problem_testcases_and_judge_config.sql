alter table oj.problems
    add column if not exists judge_config jsonb;

create table if not exists oj.problem_testcases (
    problem_id varchar(128) not null references oj.problems(problem_id) on delete cascade,
    case_no integer not null,
    input_data text not null default '',
    expected_output text not null default '',
    is_sample boolean not null default false,
    score integer not null default 0,
    primary key (problem_id, case_no)
);

create index if not exists idx_oj_problem_testcases_problem_id
    on oj.problem_testcases (problem_id);
