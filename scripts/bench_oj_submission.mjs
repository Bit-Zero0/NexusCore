#!/usr/bin/env node

const baseUrl = process.argv[2] ?? 'http://127.0.0.1:8080'
const iterations = Number(process.argv[3] ?? 5)
const requestedProblemId = process.argv[4]

const cppSource = `#include <bits/stdc++.h>
using namespace std;
int main() {
  long long a, b;
  if (!(cin >> a >> b)) return 0;
  cout << (a + b) << "\\n";
  return 0;
}
`

const terminalStatuses = new Set([
  'accepted',
  'wrong_answer',
  'compile_error',
  'runtime_error',
])

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

async function resolveProblemId() {
  if (requestedProblemId) {
    return requestedProblemId
  }

  const problems = await request('/api/v1/oj/problems')
  const fallback = problems.find((problem) => problem.judge_mode === 'acm')
  if (!fallback) {
    throw new Error('no ACM problem available for benchmark')
  }
  return fallback.problem_id
}

async function runOnce(index, problemId) {
  const startedAt = Date.now()
  const submission = await request('/api/v1/oj/submissions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      problem_id: problemId,
      user_id: 'bench-user',
      language: 'cpp',
      source_code: cppSource,
    }),
  })

  let detail = null
  for (let i = 0; i < 200; i += 1) {
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
    run: index,
    submissionId: submission.submission_id,
    status: detail?.submission?.status ?? 'unknown',
    e2eMs: finishedAt - startedAt,
    resultMs: detail?.result?.time_used_ms ?? 0,
    compileMs,
    caseMs,
  }
}

function summarize(results) {
  const values = (key) => results.map((item) => item[key])
  const avg = (items) =>
    items.length === 0 ? 0 : Math.round(items.reduce((sum, item) => sum + item, 0) / items.length)

  return {
    runs: results.length,
    avgE2eMs: avg(values('e2eMs')),
    avgResultMs: avg(values('resultMs')),
    avgCompileMs: avg(values('compileMs')),
    avgCaseMs: avg(values('caseMs')),
    maxE2eMs: Math.max(...values('e2eMs')),
    minE2eMs: Math.min(...values('e2eMs')),
  }
}

const results = []
const problemId = await resolveProblemId()
for (let i = 1; i <= iterations; i += 1) {
  const result = await runOnce(i, problemId)
  results.push(result)
  console.log(JSON.stringify(result))
}

console.log(JSON.stringify({ problemId, summary: summarize(results) }))
