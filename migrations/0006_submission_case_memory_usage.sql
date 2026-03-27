alter table oj.submission_case_results
    add column if not exists memory_used_kb bigint not null default 0;
