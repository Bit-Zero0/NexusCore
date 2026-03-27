#!/usr/bin/env node

const baseUrl = process.argv[2] ?? 'http://127.0.0.1:8080'
const iterations = Number(process.argv[3] ?? 5)

const terminalStatuses = new Set([
  'accepted',
  'wrong_answer',
  'compile_error',
  'runtime_error',
])

const cppSource = `#include <cstdio>

int main() {
  int a = 0;
  int b = 0;
  if (scanf("%d %d", &a, &b) != 2) {
    return 1;
  }
  printf("%d\\n", a + b);
  return 0;
}
`

const rustSource = `use std::io::{self, Read};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let nums: Vec<i32> = input
        .split_whitespace()
        .map(|item| item.parse::<i32>().unwrap())
        .collect();
    println!("{}", nums[0] + nums[1]);
}
`

const cases = [
  {
    language: 'cpp',
    sandbox_kind: 'nsjail',
    problem_id: 'bench-cpp-nsjail',
    slug: 'bench-cpp-nsjail',
    title: 'C++ nsjail bench',
    source_code: cppSource,
  },
  {
    language: 'cpp',
    sandbox_kind: 'wasm',
    problem_id: 'bench-cpp-wasm',
    slug: 'bench-cpp-wasm',
    title: 'C++ wasm bench',
    source_code: cppSource,
  },
  {
    language: 'rust',
    sandbox_kind: 'nsjail',
    problem_id: 'bench-rust-nsjail',
    slug: 'bench-rust-nsjail',
    title: 'Rust nsjail bench',
    source_code: rustSource,
  },
  {
    language: 'rust',
    sandbox_kind: 'wasm',
    problem_id: 'bench-rust-wasm',
    slug: 'bench-rust-wasm',
    title: 'Rust wasm bench',
    source_code: rustSource,
  },
]

async function request(path, init) {
  const response = await fetch(`${baseUrl}${path}`, init)
  const text = await response.text()
  const data = text ? JSON.parse(text) : null
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}: ${text}`)
  }
  return data
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

async function ensureProblem(definition) {
  const payload = {
    problem_id: definition.problem_id,
    title: definition.title,
    slug: definition.slug,
    judge_mode: 'acm',
    sandbox_kind: definition.sandbox_kind,
    statement_md: 'benchmark problem',
    supported_languages: [definition.language],
    limits: {
      [definition.language]: {
        time_limit_ms: 1000,
        memory_limit_kb: 262144,
      },
    },
    testcases: [
      {
        case_no: 1,
        input: '1 2\n',
        expected_output: '3\n',
        is_sample: true,
        score: 100,
      },
    ],
    judge_config: null,
    easy_config: null,
  }

  try {
    await request('/api/v1/oj/problems', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    })
  } catch (error) {
    if (!String(error).includes('409')) {
      throw error
    }

    await request(`/api/v1/oj/problems/${definition.problem_id}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    })
  }
}

async function runOnce(definition, runIndex) {
  const startedAt = Date.now()
  const submission = await request('/api/v1/oj/submissions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      problem_id: definition.problem_id,
      user_id: 'bench-user',
      language: definition.language,
      source_code: definition.source_code,
    }),
  })

  let detail = null
  for (let i = 0; i < 300; i += 1) {
    detail = await request(`/api/v1/oj/submissions/${submission.submission_id}`)
    if (terminalStatuses.has(detail.submission.status)) {
      break
    }
    await sleep(50)
  }

  const finishedAt = Date.now()
  const task = await request(`/api/v1/runtime/tasks/task-${submission.submission_id}`)
  const compileMs = task.outcome?.compile?.duration_ms ?? 0
  const caseMs = (task.outcome?.cases ?? []).reduce(
    (sum, item) => sum + (item.duration_ms ?? 0),
    0,
  )

  return {
    language: definition.language,
    sandbox_kind: definition.sandbox_kind,
    run: runIndex,
    submission_id: submission.submission_id,
    status: detail?.submission?.status ?? 'unknown',
    e2e_ms: finishedAt - startedAt,
    compile_ms: compileMs,
    run_ms: detail?.result?.run_time_ms ?? 0,
    result_ms: detail?.result?.time_used_ms ?? 0,
    memory_kb: detail?.result?.memory_used_kb ?? 0,
    case_ms: caseMs,
  }
}

function summarize(results) {
  const avg = (items) =>
    items.length === 0 ? 0 : Math.round(items.reduce((sum, item) => sum + item, 0) / items.length)
  const values = (key) => results.map((item) => item[key])
  return {
    runs: results.length,
    avg_e2e_ms: avg(values('e2e_ms')),
    avg_compile_ms: avg(values('compile_ms')),
    avg_run_ms: avg(values('run_ms')),
    avg_memory_kb: avg(values('memory_kb')),
    min_e2e_ms: Math.min(...values('e2e_ms')),
    max_e2e_ms: Math.max(...values('e2e_ms')),
  }
}

const allResults = []
for (const definition of cases) {
  await ensureProblem(definition)
  for (let i = 1; i <= iterations; i += 1) {
    const result = await runOnce(definition, i)
    allResults.push(result)
    console.log(JSON.stringify(result))
  }
}

const grouped = Object.groupBy(
  allResults,
  (item) => `${item.language}:${item.sandbox_kind}`,
)
const summary = Object.fromEntries(
  Object.entries(grouped).map(([key, value]) => [key, summarize(value)]),
)

console.log(JSON.stringify({ iterations, summary }, null, 2))
