function normalizeApiBaseUrl(value?: string): string {
  const raw = value?.trim()
  if (!raw || raw === '/api') {
    return ''
  }
  return raw.endsWith('/') ? raw.slice(0, -1) : raw
}

const API_BASE = normalizeApiBaseUrl(import.meta.env.VITE_NEXUS_GATE_URL)
const API_TOKEN = import.meta.env.VITE_NEXUS_GATE_TOKEN ?? ''
const API_USER_ID = import.meta.env.VITE_NEXUS_USER_ID ?? 'dev-user'

type JudgeMode = 'acm' | 'easy' | 'functional'
type JudgeMethod = 'validator' | 'spj'

interface BackendProblemLimits {
  time_limit_ms: number
  memory_limit_kb: number
}

interface BackendTestcase {
  case_no: number
  input: string
  expected_output: string
  is_sample: boolean
  score: number
}

interface BackendProblem {
  problem_id: string
  title: string
  slug: string
  judge_mode: 'acm' | 'functional' | 'easy_judge'
  sandbox_kind: 'nsjail' | 'wasm' | 'nsjail_wasm'
  statement_md: string
  supported_languages: string[]
  limits: Record<string, BackendProblemLimits>
  testcases: BackendTestcase[]
  judge_config: {
    judge_method: 'validator' | 'spj'
    validator?: Record<string, unknown> | null
    spj?: { language: string; source_code: string } | null
    function_signature?: Record<string, unknown> | null
  } | null
  easy_config: {
    question_type: 'true_false' | 'single_choice' | 'multiple_choice'
    options: Array<{ key: string; label: string }>
    standard_answer: string | string[]
  } | null
}

interface BackendProblemSummary {
  problem_id: string
  title: string
  slug: string
  judge_mode: 'acm' | 'functional' | 'easy_judge'
}

interface BackendProblemDetail {
  problem: BackendProblem
}

interface BackendSubmissionRecord {
  submission_id: string
  problem_id: string
  user_id: string
  language: string
  status: string
  score: number
  max_score: number
  message: string | null
}

interface BackendSubmissionCaseResult {
  case_no: number
  status: 'accepted' | 'wrong_answer' | 'runtime_error'
  score: number
  time_used_ms: number
  memory_used_kb: number
  actual_output: string
  expected_output_snapshot: string
  message: string | null
}

interface BackendSubmissionResult {
  submission_id: string
  overall_status: string
  compile_output: string | null
  runtime_output: string | null
  compile_time_ms: number
  judge_compile_time_ms: number
  run_time_ms: number
  time_used_ms: number
  memory_used_kb: number
  judge_summary: string | null
  case_results: BackendSubmissionCaseResult[]
}

interface BackendSubmissionDetail {
  submission: BackendSubmissionRecord
  source_code: string
  result: BackendSubmissionResult | null
}

interface BackendRuntimeTask {
  task_id: string
  queue: string
  lane: string
}

interface BackendRuntimeNodeStatus {
  node_id: string
  started_at_ms: number
  last_heartbeat_ms: number
  node_status: 'healthy' | 'stale'
  worker_groups: Array<{
    name: string
    bindings: Array<{ queue: string; lane: string }>
  }>
}

interface BackendRuntimeNodeSummary {
  total_nodes: number
  healthy_nodes: number
  stale_nodes: number
  routes: Array<{
    queue: string
    lane: string
    node_count: number
    worker_group_count: number
  }>
  groups: Array<{
    name: string
    node_count: number
    binding_count: number
  }>
}

export interface ApiProblem {
  problem_id: string
  title: string
  description: string
  statement_md?: string
  input_desc_md?: string
  output_desc_md?: string
  samples_json?: Array<Record<string, unknown>>
  judge_mode: JudgeMode
  judge_queue?: '' | 'fast' | 'normal' | 'heavy'
  judge_method?: JudgeMethod
  sandbox_kind?: 'nsjail' | 'wasm' | 'nsjail_wasm'
  judge_config?: Record<string, unknown>
  spj_source_code?: string
  spj_language?: string
  languages: string[]
  tags: string[]
  testcases?: Array<Record<string, unknown>>
  easy_judger?: Record<string, unknown>
  function_details_json?: Record<string, unknown>
  resource_limits?: Record<string, { time_limit_ms: number; memory_limit_kb: number }>
}

export interface ApiSubmission {
  submission_id: string
  problem_id: string
  problem_title: string
  judge_mode: string
  language: string
  source_code: string
  route_lane: string
  route_queue: string
  status: string
  score: number
  max_score: number
  time_used_ms: number | null
  memory_used_kb: number | null
  compile_time_ms: number | null
  judge_compile_time_ms: number | null
  run_time_ms: number | null
  error_message: string
  judge_summary: Record<string, unknown>
  created_at_ms: number
  updated_at_ms: number
}

export interface ApiClusterNode {
  node_id: string
  status: 'online' | 'unhealthy' | string
  utilization: number
  active_tasks: number
  capacity: number
  ttl_sec?: number
  last_heartbeat_ms?: number
  supported_lanes?: string[] | string
}

export interface ApiClusterStats {
  online_nodes: number
  busy_nodes: number
  unhealthy_nodes: number
  total_nodes: number
  avg_utilization: number
  nodes: ApiClusterNode[]
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers ?? {})
  if (!headers.has('Content-Type') && init.body) {
    headers.set('Content-Type', 'application/json')
  }
  if (API_TOKEN && API_TOKEN !== 'replace-me' && !headers.has('Authorization')) {
    headers.set('Authorization', `Bearer ${API_TOKEN}`)
  }

  const response = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers,
  })

  const data = await response.json().catch(() => null)
  if (!response.ok) {
    const message =
      (data && typeof data === 'object' && 'message' in data && typeof data.message === 'string' && data.message) ||
      (data && typeof data === 'object' && 'error' in data && typeof data.error === 'string' && data.error) ||
      `request failed: ${response.status}`
    throw new Error(message)
  }
  return data as T
}

function mapJudgeMode(mode: BackendProblem['judge_mode']): JudgeMode {
  return mode === 'easy_judge' ? 'easy' : mode
}

function mapJudgeMethod(problem: BackendProblem): JudgeMethod {
  const method = problem.judge_config?.judge_method
  if (method === 'spj') return 'spj'
  return 'validator'
}

function inferJudgeQueue(problem: BackendProblem): '' | 'fast' | 'normal' | 'heavy' {
  if (problem.judge_mode === 'functional') return 'heavy'
  if (problem.supported_languages.includes('python')) return 'normal'
  return 'fast'
}

function mapProblem(problem: BackendProblem | BackendProblemSummary): ApiProblem {
  const full = 'statement_md' in problem
  const fullProblem = full ? problem : null
  const easyConfig = fullProblem?.easy_config
  const questionType = easyConfig?.question_type ?? 'single_choice'
  const judgeConfig =
    fullProblem?.judge_config?.judge_method === 'validator'
      ? (fullProblem.judge_config.validator as Record<string, unknown> | null)
      : undefined

  return {
    problem_id: problem.problem_id,
    title: problem.title,
    description: fullProblem?.statement_md ?? '',
    statement_md: fullProblem?.statement_md ?? '',
    input_desc_md: fullProblem && problem.judge_mode === 'functional'
      ? '请按照函数签名实现目标函数，参数将由评测系统自动注入。'
      : undefined,
    output_desc_md: fullProblem && problem.judge_mode !== 'easy_judge'
      ? '输出需与标准答案匹配。'
      : undefined,
    samples_json: fullProblem?.testcases
      ?.filter((testcase) => testcase.is_sample)
      .map((testcase) => ({
        input: testcase.input,
        output: testcase.expected_output,
      })),
    judge_mode: mapJudgeMode(problem.judge_mode),
    judge_queue: fullProblem ? inferJudgeQueue(fullProblem) : '',
    judge_method: fullProblem ? mapJudgeMethod(fullProblem) : 'validator',
    sandbox_kind: fullProblem?.sandbox_kind ?? 'nsjail',
    judge_config: judgeConfig ?? undefined,
    spj_source_code: fullProblem?.judge_config?.spj?.source_code ?? undefined,
    spj_language: fullProblem?.judge_config?.spj?.language ?? undefined,
    languages: fullProblem?.supported_languages ?? [],
    tags: [],
    testcases: fullProblem?.testcases?.map((testcase) => ({
      case_no: testcase.case_no,
      input: testcase.input,
      output: testcase.expected_output,
      expected_output: testcase.expected_output,
      is_sample: testcase.is_sample,
      score: testcase.score,
    })),
    easy_judger: easyConfig
      ? {
          question_type: questionType,
          standard_answer: easyConfig.standard_answer,
          metadata: {
            options: easyConfig.options.map((option) => option.key),
            option_descriptions: Object.fromEntries(
              easyConfig.options.map((option) => [option.key, option.label]),
            ),
            full_score: 100,
            min_selections: questionType === 'multiple_choice' ? 1 : 1,
            max_selections:
              questionType === 'multiple_choice' ? easyConfig.options.length : 1,
          },
        }
      : undefined,
    function_details_json:
      fullProblem?.judge_config?.function_signature ?? undefined,
    resource_limits: fullProblem?.limits ?? {},
  }
}

function parseSubmissionTime(submissionId: string): number {
  const match = submissionId.match(/sub-(\d+)/)
  return match ? Number(match[1]) : Date.now()
}

function mapCaseStatusToCode(status: BackendSubmissionCaseResult['status']): number {
  switch (status) {
    case 'accepted':
      return 2
    case 'wrong_answer':
      return 3
    case 'runtime_error':
      return 6
    default:
      return 1
  }
}

function mapSubmission(
  detail: BackendSubmissionDetail,
  problemTitle?: string,
  task?: BackendRuntimeTask | null,
): ApiSubmission {
  const createdAt = parseSubmissionTime(detail.submission.submission_id)
  const result = detail.result

  return {
    submission_id: detail.submission.submission_id,
    problem_id: detail.submission.problem_id,
    problem_title: problemTitle ?? detail.submission.problem_id,
    judge_mode: 'standard',
    language: detail.submission.language,
    source_code: detail.source_code,
    route_lane: task?.lane ?? '--',
    route_queue: task?.queue ?? '--',
    status: detail.submission.status,
    score: detail.submission.score,
    max_score: detail.submission.max_score,
    time_used_ms: result?.time_used_ms ?? null,
    memory_used_kb: result?.memory_used_kb ?? null,
    compile_time_ms: result?.compile_time_ms ?? null,
    judge_compile_time_ms: result?.judge_compile_time_ms ?? null,
    run_time_ms: result?.run_time_ms ?? null,
    error_message:
      detail.submission.message ?? result?.runtime_output ?? result?.compile_output ?? '',
    judge_summary: {
      compile_output: result?.compile_output ?? '',
      runtime_output: result?.runtime_output ?? '',
      summary: result?.judge_summary ?? '',
      compile_time_ms: result?.compile_time_ms ?? 0,
      judge_compile_time_ms: result?.judge_compile_time_ms ?? 0,
      run_time_ms: result?.run_time_ms ?? 0,
      test_results:
        result?.case_results.map((item) => ({
          status: mapCaseStatusToCode(item.status),
          time_ms: item.time_used_ms,
          memory_kb: item.memory_used_kb,
          stdout_output: item.actual_output,
          stderr_output: item.message ?? '',
          exit_code: item.status === 'accepted' ? 0 : 1,
          expected_output: item.expected_output_snapshot,
        })) ?? [],
    },
    created_at_ms: createdAt,
    updated_at_ms: Date.now(),
  }
}

function slugify(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

function buildBackendProblemPayload(payload: Record<string, unknown>): BackendProblem {
  const judgeMode = String(payload.judge_mode ?? 'acm') as JudgeMode
  const languages = Array.isArray(payload.languages)
    ? payload.languages.map((item) => String(item))
    : []
  const resourceLimits = ((payload.resource_limits ?? {}) as Record<
    string,
    { time_limit_ms?: number; memory_limit_kb?: number }
  >)
  const testcases = Array.isArray(payload.testcases)
    ? payload.testcases.map((item, index) => {
        const row = item as Record<string, unknown>
        return {
          case_no: Number(row.case_no ?? index + 1),
          input: String(row.input ?? ''),
          expected_output: String(row.output ?? row.expected_output ?? ''),
          is_sample: Boolean(row.is_sample ?? index < 2),
          score: Number(row.score ?? 0) || 100,
        }
      })
    : []

  const backendJudgeMode: BackendProblem['judge_mode'] =
    judgeMode === 'easy' ? 'easy_judge' : judgeMode
  const problemId = String(payload.problem_id ?? '')
  const title = String(payload.title ?? '')
  const statement = String(payload.statement_md ?? payload.description ?? '')
  const judgeMethod = String(payload.judge_method ?? 'validator')
  const sandboxKind = String(payload.sandbox_kind ?? 'nsjail')
  const validator = (payload.judge_config ?? {}) as Record<string, unknown>
  const functionDetails = (payload.function_details_json ?? null) as Record<string, unknown> | null

  const judgeConfig =
    judgeMode === 'easy'
      ? null
      : {
          judge_method: (judgeMethod === 'spj' ? 'spj' : 'validator') as 'validator' | 'spj',
          validator:
            judgeMethod === 'spj'
              ? null
              : {
                  ignore_whitespace: Boolean(validator.ignore_whitespace ?? true),
                  ignore_case: Boolean(validator.ignore_case ?? false),
                  is_unordered: Boolean(validator.is_unordered ?? false),
                  is_token_mode: Boolean(validator.is_token_mode ?? false),
                  is_float: Boolean(validator.is_float ?? false),
                  float_epsilon: Number(validator.float_epsilon ?? 0),
                },
          spj:
            judgeMethod === 'spj'
              ? {
                  language: String(payload.spj_language ?? 'cpp'),
                  source_code: String(payload.spj_source_code ?? ''),
                }
              : null,
          function_signature:
            judgeMode === 'functional' && functionDetails ? functionDetails : null,
        }

  const easyJudger = (payload.easy_judger ?? null) as
    | {
        question_type?: string
        standard_answer?: unknown
        metadata?: Record<string, unknown>
      }
    | null

  const easyConfig =
    judgeMode === 'easy' && easyJudger
      ? {
          question_type: (String(easyJudger.question_type ?? 'single_choice') as
            | 'true_false'
            | 'single_choice'
            | 'multiple_choice'),
          options: Array.isArray(easyJudger.metadata?.options)
            ? easyJudger.metadata.options.map((option) => ({
                key: String(option),
                label: String(
                  ((easyJudger.metadata?.option_descriptions as Record<string, unknown> | undefined) ??
                    {})[String(option)] ?? String(option),
                ),
              }))
            : [],
          standard_answer:
            Array.isArray(easyJudger.standard_answer)
              ? easyJudger.standard_answer.map((item) => String(item))
              : String(easyJudger.standard_answer ?? ''),
        }
      : null

  return {
    problem_id: problemId,
    title,
    slug: slugify(problemId || title),
    judge_mode: backendJudgeMode,
    sandbox_kind:
      sandboxKind === 'wasm' || sandboxKind === 'nsjail_wasm'
        ? (sandboxKind as 'wasm' | 'nsjail_wasm')
        : 'nsjail',
    statement_md: statement,
    supported_languages: judgeMode === 'easy' ? [] : languages,
    limits: judgeMode === 'easy'
      ? {}
      : Object.fromEntries(
          languages.map((language) => [
            language,
            {
              time_limit_ms: Number(resourceLimits[language]?.time_limit_ms ?? (language === 'python' ? 2000 : 1000)),
              memory_limit_kb: Number(resourceLimits[language]?.memory_limit_kb ?? (language === 'python' ? 102400 : 51200)),
            },
          ]),
        ),
    testcases: judgeMode === 'easy' ? [] : testcases,
    judge_config: judgeConfig,
    easy_config: easyConfig,
  }
}

async function tryGetRuntimeTask(submissionId: string): Promise<BackendRuntimeTask | null> {
  try {
    return await request<BackendRuntimeTask>(`/api/v1/oj/submissions/${submissionId}/runtime-task`)
  } catch {
    return null
  }
}

async function tryGetProblemTitle(problemId: string): Promise<string | undefined> {
  try {
    const detail = await request<BackendProblemDetail>(`/api/v1/oj/problems/${problemId}`)
    return detail.problem.title
  } catch {
    return undefined
  }
}

export const api = {
  baseUrl: API_BASE,
  token: API_TOKEN,
  listProblems: async () => {
    const problems = await request<BackendProblemSummary[]>('/api/v1/oj/problems')
    return { problems: problems.map(mapProblem) }
  },
  getProblem: async (problemId: string) => {
    const detail = await request<BackendProblemDetail>(`/api/v1/oj/problems/${problemId}`)
    return { problem: mapProblem(detail.problem) }
  },
  createProblem: async (payload: Record<string, unknown>) => {
    const detail = await request<BackendProblemDetail>('/api/v1/oj/problems', {
      method: 'POST',
      body: JSON.stringify(buildBackendProblemPayload(payload)),
    })
    return { problem: mapProblem(detail.problem) }
  },
  validateProblem: async (payload: Record<string, unknown>) => {
    return buildBackendProblemPayload(payload)
  },
  updateProblem: async (problemId: string, payload: Record<string, unknown>) => {
    const detail = await request<BackendProblemDetail>(`/api/v1/oj/problems/${problemId}`, {
      method: 'PUT',
      body: JSON.stringify(buildBackendProblemPayload(payload)),
    })
    return { problem: mapProblem(detail.problem) }
  },
  createSubmission: async (payload: { problem_id: string; language: string; source_code: string }) => {
    const record = await request<BackendSubmissionRecord>('/api/v1/oj/submissions', {
      method: 'POST',
      body: JSON.stringify({
        ...payload,
        user_id: API_USER_ID,
      }),
    })
    const task = await tryGetRuntimeTask(record.submission_id)
    return {
      submission_id: record.submission_id,
      queue: task?.queue ?? 'oj_judge',
      route_lane: task?.lane ?? '--',
      route_reason: 'scheduled_by_oj',
      route_degraded: false,
    }
  },
  createEasySubmission: async (payload: { problem_id: string; answer: unknown }) => {
    const record = await request<BackendSubmissionRecord>('/api/v1/oj/easy-judge/submissions', {
      method: 'POST',
      body: JSON.stringify({
        ...payload,
        user_id: API_USER_ID,
      }),
    })
    return {
      source: 'oj_easy_judge',
      submission_id: record.submission_id,
      question_id: record.problem_id,
      result: {
        status: record.status,
        score: record.score,
        max_score: record.max_score,
        message: record.message,
      },
    }
  },
  listSubmissions: async () => {
    const [submissions, problems] = await Promise.all([
      request<BackendSubmissionRecord[]>('/api/v1/oj/submissions'),
      request<BackendProblemSummary[]>('/api/v1/oj/problems').catch(() => []),
    ])
    const titleMap = new Map(problems.map((problem) => [problem.problem_id, problem.title]))
    return {
      submissions: submissions.map((item) => ({
        submission_id: item.submission_id,
        problem_id: item.problem_id,
        problem_title: titleMap.get(item.problem_id) ?? item.problem_id,
        judge_mode: 'acm',
        language: item.language,
        source_code: '',
        route_lane: '--',
        route_queue: '--',
        status: item.status,
        score: item.score,
        max_score: item.max_score,
        time_used_ms: null,
        memory_used_kb: null,
        compile_time_ms: null,
        judge_compile_time_ms: null,
        run_time_ms: null,
        error_message: item.message ?? '',
        judge_summary: {},
        created_at_ms: parseSubmissionTime(item.submission_id),
        updated_at_ms: Date.now(),
      })),
    }
  },
  getSubmission: async (submissionId: string) => {
    const detail = await request<BackendSubmissionDetail>(`/api/v1/oj/submissions/${submissionId}`)
    const [task, problemTitle] = await Promise.all([
      tryGetRuntimeTask(submissionId),
      tryGetProblemTitle(detail.submission.problem_id),
    ])
    return {
      submission: mapSubmission(detail, problemTitle, task),
    }
  },
  getClusterStats: async () => {
    const [summary, nodes] = await Promise.all([
      request<BackendRuntimeNodeSummary>('/api/v1/runtime/nodes/summary'),
      request<BackendRuntimeNodeStatus[]>('/api/v1/runtime/nodes'),
    ])

    const mappedNodes: ApiClusterNode[] = nodes.map((node) => ({
      node_id: node.node_id,
      status: node.node_status === 'healthy' ? 'online' : 'unhealthy',
      utilization: 0,
      active_tasks: 0,
      capacity: node.worker_groups.length,
      ttl_sec: undefined,
      last_heartbeat_ms: node.last_heartbeat_ms,
      supported_lanes: [...new Set(node.worker_groups.flatMap((group) => group.bindings.map((binding) => `${binding.queue}:${binding.lane}`)))],
    }))

    return {
      online_nodes: summary.healthy_nodes,
      busy_nodes: summary.groups.length,
      unhealthy_nodes: summary.stale_nodes,
      total_nodes: summary.total_nodes,
      avg_utilization: 0,
      nodes: mappedNodes,
    } satisfies ApiClusterStats
  },
  makeSubmissionWsUrl(submissionId: string) {
    const wsBase = API_BASE.replace(/^http/, 'ws')
    return `${wsBase}/ws/v1/submission/${submissionId}`
  },
}
