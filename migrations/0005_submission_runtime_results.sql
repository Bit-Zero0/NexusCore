alter table oj.submissions
    add column if not exists updated_at timestamptz not null default now(),
    add column if not exists execution_summary jsonb;

create table if not exists oj.submission_case_results (
    submission_id varchar(128) not null references oj.submissions(submission_id) on delete cascade,
    case_no integer not null,
    score integer not null default 0,
    status varchar(32) not null,
    exit_code integer,
    duration_ms bigint not null default 0,
    stdout_path text not null default '',
    stderr_path text not null default '',
    stdout_excerpt text not null default '',
    stderr_excerpt text not null default '',
    primary key (submission_id, case_no)
);

alter table oj.submission_case_results
    add column if not exists score integer not null default 0;

create index if not exists idx_oj_submission_case_results_submission_id
    on oj.submission_case_results (submission_id);
