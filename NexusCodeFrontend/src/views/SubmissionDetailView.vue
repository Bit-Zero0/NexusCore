<script setup lang="ts">
import { defineAsyncComponent, computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'

import { api, type ApiProblem, type ApiSubmission } from '../lib/api'

const CodeEditor = defineAsyncComponent(() => import('../components/CodeEditor.vue'))

const route = useRoute()
const submission = ref<ApiSubmission | null>(null)
const problem = ref<ApiProblem | null>(null)
const error = ref('')
const copyState = ref<'idle' | 'success' | 'error'>('idle')
const wsState = ref<'connecting' | 'open' | 'closed' | 'error'>('connecting')
let pollingTimer: number | null = null
let copyTimer: number | null = null

const stopPolling = () => {
  if (pollingTimer !== null) {
    window.clearInterval(pollingTimer)
    pollingTimer = null
  }
}

const loadSubmission = async () => {
  try {
    const response = await api.getSubmission(String(route.params.submissionId))
    submission.value = response.submission
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载提交详情失败'
  }
}

const loadProblem = async (problemId: string) => {
  try {
    const response = await api.getProblem(problemId)
    problem.value = response.problem
  } catch {
    problem.value = null
  }
}

const mapOverallStatus = (status: number) => {
  switch (status) {
    case 2:
      return 'accepted'
    case 3:
      return 'wrong_answer'
    case 4:
      return 'time_limit_exceeded'
    case 5:
      return 'memory_limit_exceeded'
    case 6:
      return 'runtime_error'
    case 7:
      return 'compile_error'
    case 8:
      return 'internal_error'
    case 9:
      return 'dangerous_syscall'
    default:
      return 'processing'
  }
}

const sanitizeErrorOutput = (raw: string) => {
  const trimmed = raw.trim()
  if (!trimmed) return ''

  const lines = trimmed
    .split('\n')
    .map((line) => line.trimEnd())
    .filter(Boolean)

  const signalLines = lines.filter((line) => {
    const lower = line.toLowerCase()
    return (
      lower.includes('error') ||
      lower.includes('exception') ||
      lower.includes('traceback') ||
      lower.includes('segmentation fault') ||
      lower.includes('killed') ||
      lower.includes('terminated') ||
      lower.includes('syntaxerror') ||
      lower.includes('nameerror') ||
      lower.includes('typeerror') ||
      lower.includes('valueerror') ||
      lower.includes('assert') ||
      lower.includes('abort') ||
      lower.includes('runtime')
    )
  })

  if (signalLines.length > 0) {
    return signalLines.slice(0, 12).join('\n')
  }

  const nonSandboxLines = lines.filter((line) => {
    const lower = line.toLowerCase()
    return !(
      lower.startsWith('[d]') ||
      lower.startsWith('[i]') ||
      lower.startsWith('[w]') ||
      lower.startsWith('[debug]') ||
      lower.includes('nsjail') ||
      lower.includes('mount:') ||
      lower.includes('clone flags') ||
      lower.includes('cgroup') ||
      lower.includes('capabilities') ||
      lower.includes('setsighandler') ||
      lower.includes('remounting') ||
      lower.includes('uid map') ||
      lower.includes('gid map')
    )
  })

  const source = nonSandboxLines.length > 0 ? nonSandboxLines : lines
  return source.slice(0, 12).join('\n')
}

const collapseLongText = (raw: string, head = 12, tail = 8) => {
  const normalized = raw.replace(/\r\n/g, '\n')
  const lines = normalized.split('\n')
  if (lines.length <= head + tail + 1) {
    return lines
  }
  return [
    ...lines.slice(0, head),
    `... 省略 ${lines.length - head - tail} 行 ...`,
    ...lines.slice(-tail),
  ]
}

const truncateMiddle = (value: string, keepStart = 80, keepEnd = 40) => {
  if (value.length <= keepStart + keepEnd + 12) {
    return value
  }
  return `${value.slice(0, keepStart)} ...[省略 ${value.length - keepStart - keepEnd} 字符]... ${value.slice(-keepEnd)}`
}

const buildCharSegments = (expected: string, actual: string) => {
  const leftExpected = truncateMiddle(expected)
  const leftActual = truncateMiddle(actual)
  let prefix = 0
  const maxPrefix = Math.min(leftExpected.length, leftActual.length)
  while (prefix < maxPrefix && leftExpected[prefix] === leftActual[prefix]) {
    prefix += 1
  }

  let suffix = 0
  const expectedRemain = leftExpected.length - prefix
  const actualRemain = leftActual.length - prefix
  const maxSuffix = Math.min(expectedRemain, actualRemain)
  while (
    suffix < maxSuffix &&
    leftExpected[leftExpected.length - 1 - suffix] === leftActual[leftActual.length - 1 - suffix]
  ) {
    suffix += 1
  }

  const expectedMiddle = leftExpected.slice(prefix, leftExpected.length - suffix)
  const actualMiddle = leftActual.slice(prefix, leftActual.length - suffix)
  const sharedPrefix = leftExpected.slice(0, prefix)
  const sharedSuffix = suffix > 0 ? leftExpected.slice(leftExpected.length - suffix) : ''

  const makeSegments = (middle: string) =>
    [
      sharedPrefix ? { text: sharedPrefix, changed: false } : null,
      middle ? { text: middle, changed: true } : null,
      sharedSuffix ? { text: sharedSuffix, changed: false } : null,
    ].filter(Boolean) as Array<{ text: string; changed: boolean }>

  return {
    expectedSegments: makeSegments(expectedMiddle),
    actualSegments: makeSegments(actualMiddle),
  }
}

const buildLineDiff = (expected: string, actual: string) => {
  const expectedLines = collapseLongText(expected)
  const actualLines = collapseLongText(actual)
  const max = Math.max(expectedLines.length, actualLines.length)

  return Array.from({ length: max }, (_, index) => {
    const expectedLine = expectedLines[index] ?? ''
    const actualLine = actualLines[index] ?? ''
    let kind: 'same' | 'changed' | 'missing' | 'extra' = 'same'

    if (expectedLine === actualLine) {
      kind = 'same'
    } else if (expectedLine && !actualLine) {
      kind = 'missing'
    } else if (!expectedLine && actualLine) {
      kind = 'extra'
    } else {
      kind = 'changed'
    }

    const { expectedSegments, actualSegments } = buildCharSegments(expectedLine, actualLine)

    return {
      lineNo: index + 1,
      expected: expectedLine,
      actual: actualLine,
      kind,
      expectedSegments,
      actualSegments,
    }
  })
}

const isTerminalStatus = (status: string | undefined) =>
  ['accepted', 'wrong_answer', 'compile_error', 'runtime_error'].includes(status ?? '')

const startPolling = () => {
  stopPolling()
  wsState.value = 'open'
  pollingTimer = window.setInterval(async () => {
    try {
      await loadSubmission()
      if (isTerminalStatus(submission.value?.status)) {
        wsState.value = 'closed'
        stopPolling()
      }
    } catch {
      wsState.value = 'error'
      stopPolling()
    }
  }, 2000)
}

onMounted(async () => {
  await loadSubmission()
  if (isTerminalStatus(submission.value?.status)) {
    wsState.value = 'closed'
    return
  }
  startPolling()
})

watch(
  () => submission.value?.problem_id,
  async (problemId) => {
    if (!problemId) return
    await loadProblem(problemId)
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  stopPolling()
  if (copyTimer !== null) {
    window.clearTimeout(copyTimer)
    copyTimer = null
  }
})

const currentSubmission = computed(() => submission.value)

const compileMessage = computed(() => {
  const compileOutput = currentSubmission.value?.judge_summary?.compile_output
  if (typeof compileOutput === 'string' && compileOutput.trim()) {
    return collapseLongText(compileOutput).join('\n')
  }
  if (currentSubmission.value?.status === 'compile_error' && currentSubmission.value.error_message) {
    return collapseLongText(currentSubmission.value.error_message).join('\n')
  }
  return ''
})

const testcaseRows = computed(() => {
  const rawResults = currentSubmission.value?.judge_summary?.test_results
  if (!Array.isArray(rawResults)) return []

  return rawResults.map((item, index) => {
    const row = item as Record<string, unknown>
    const expected = Array.isArray(problem.value?.testcases)
      ? String((problem.value?.testcases[index]?.output as string | undefined) ?? '')
      : ''

    return {
      name: `case_${String(index + 1).padStart(2, '0')}`,
      status: mapOverallStatus(Number(row.status ?? 0)),
      time: row.time_ms ?? '--',
      memory: row.memory_kb ?? '--',
      expected,
      actual: String(row.stdout_output ?? ''),
      stderr: String(row.stderr_output ?? ''),
      exitCode: row.exit_code ?? '--',
    }
  })
})

const firstFailedCase = computed(() => testcaseRows.value.find((row) => row.status !== 'accepted') ?? null)

const runtimeMessage = computed(() => {
  if (!firstFailedCase.value) return ''
  if (firstFailedCase.value.status !== 'runtime_error') return ''
  return collapseLongText(
    sanitizeErrorOutput(firstFailedCase.value.stderr || currentSubmission.value?.error_message || ''),
  ).join('\n')
})

const diffSummary = computed(() => {
  if (!firstFailedCase.value) return null
  if (firstFailedCase.value.status !== 'wrong_answer') return null
  return {
    expected: firstFailedCase.value.expected,
    actual: firstFailedCase.value.actual,
    lines: buildLineDiff(firstFailedCase.value.expected, firstFailedCase.value.actual),
  }
})

const editorLanguage = computed<'cpp' | 'python'>(() =>
  currentSubmission.value?.language === 'python' ? 'python' : 'cpp',
)

const copySourceCode = async () => {
  if (!currentSubmission.value?.source_code) return
  try {
    await navigator.clipboard.writeText(currentSubmission.value.source_code)
    copyState.value = 'success'
  } catch {
    copyState.value = 'error'
  }
  if (copyTimer !== null) {
    window.clearTimeout(copyTimer)
  }
  copyTimer = window.setTimeout(() => {
    copyState.value = 'idle'
    copyTimer = null
  }, 2200)
}
</script>

<template>
  <div class="page">
    <p v-if="error" class="muted">{{ error }}</p>
    <div class="page-header">
      <div>
        <span class="page-kicker">Realtime Judge</span>
        <h2 class="page-title">提交详情 {{ currentSubmission?.submission_id ?? route.params.submissionId }}</h2>
        <p class="page-subtitle">结果页现在只展示对排错有用的信息，不再直接暴露原始 WebSocket JSON。</p>
      </div>
    </div>

    <section class="stat-grid">
      <div class="metric-card">
        <span class="card-label">当前状态</span>
        <strong>{{ currentSubmission?.status ?? '--' }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">路由队列</span>
        <strong>{{ currentSubmission?.route_lane ?? '--' }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">状态同步</span>
        <strong>{{ wsState }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">题目</span>
        <strong>{{ currentSubmission?.problem_title ?? '--' }}</strong>
      </div>
    </section>

    <section class="section-grid">
      <div v-if="compileMessage" class="span-12 detail-card">
        <div>
          <span class="eyebrow">Compilation Error</span>
          <h3 class="section-title">编译错误信息</h3>
        </div>
        <pre class="result-block"><code>{{ compileMessage }}</code></pre>
      </div>

      <div v-else-if="runtimeMessage" class="span-12 detail-card">
        <div>
          <span class="eyebrow">Runtime Error</span>
          <h3 class="section-title">运行错误信息</h3>
        </div>
        <pre class="result-block"><code>{{ runtimeMessage }}</code></pre>
      </div>

      <div v-else-if="diffSummary" class="span-12 section-grid diff-grid-shell">
        <div class="span-12">
          <span class="eyebrow">Wrong Answer</span>
          <h3 class="section-title">预期结果与实际结果对比</h3>
        </div>
        <div class="span-12 detail-card">
          <div class="diff-header">
            <span>行号</span>
            <span>期待结果</span>
            <span>实际结果</span>
          </div>
          <div class="diff-table">
            <div
              v-for="line in diffSummary.lines"
              :key="line.lineNo"
              class="diff-row"
              :class="`diff-row-${line.kind}`"
            >
              <span class="diff-line">{{ line.lineNo }}</span>
              <code class="diff-cell">
                <template v-for="(segment, segmentIndex) in line.expectedSegments" :key="segmentIndex">
                  <span :class="{ 'char-changed': segment.changed }">{{ segment.text || ' ' }}</span>
                </template>
              </code>
              <code class="diff-cell">
                <template v-for="(segment, segmentIndex) in line.actualSegments" :key="segmentIndex">
                  <span :class="{ 'char-changed': segment.changed }">{{ segment.text || ' ' }}</span>
                </template>
              </code>
            </div>
          </div>
        </div>
      </div>

      <div v-else class="span-12 detail-card">
        <div>
          <span class="eyebrow">Judge Summary</span>
          <h3 class="section-title">运行摘要</h3>
        </div>
        <ul class="bullet-list">
          <li>当前状态：{{ currentSubmission?.status ?? '--' }}</li>
          <li>编译时间：{{ currentSubmission?.compile_time_ms ?? '--' }} ms</li>
          <li v-if="(currentSubmission?.judge_compile_time_ms ?? 0) > 0">
            判题器编译时间：{{ currentSubmission?.judge_compile_time_ms ?? '--' }} ms
          </li>
          <li>真正运行时间：{{ currentSubmission?.run_time_ms ?? currentSubmission?.time_used_ms ?? '--' }} ms</li>
          <li>内存占用：{{ currentSubmission?.memory_used_kb ?? '--' }} KB</li>
          <li v-if="currentSubmission?.error_message">错误信息：{{ currentSubmission.error_message }}</li>
        </ul>
      </div>

      <div v-if="currentSubmission?.source_code" class="span-12 detail-card">
        <div class="source-header">
          <div>
            <span class="eyebrow">Source Code</span>
            <h3 class="section-title">提交代码</h3>
          </div>
          <button class="ghost-button" type="button" @click="copySourceCode">
            {{
              copyState === 'success'
                ? '已复制'
                : copyState === 'error'
                  ? '复制失败'
                  : '复制代码'
            }}
          </button>
        </div>
        <div class="source-editor-shell">
          <CodeEditor
            :model-value="currentSubmission.source_code"
            :language="editorLanguage"
            :read-only="true"
            :font-size="14"
            :tab-size="4"
          />
        </div>
      </div>

      <div class="span-12 table-shell">
        <div class="page-header">
          <div>
            <span class="eyebrow">Testcases</span>
            <h3 class="section-title">测试点明细</h3>
          </div>
        </div>
        <table>
          <thead>
            <tr>
              <th>Case</th>
              <th>状态</th>
              <th>时间</th>
              <th>内存</th>
              <th>退出码</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="testcase in testcaseRows" :key="testcase.name">
              <td>{{ testcase.name }}</td>
              <td>
                <span
                  class="status-pill"
                  :class="
                    testcase.status === 'accepted'
                      ? 'accepted'
                      : testcase.status === 'runtime_error'
                        ? 'runtime-error'
                        : 'processing'
                  "
                >
                  {{ testcase.status }}
                </span>
              </td>
              <td>{{ testcase.time }}</td>
              <td>{{ testcase.memory }}</td>
              <td>{{ testcase.exitCode }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>

<style scoped>
.result-block {
  margin: 0;
  padding: 18px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
  color: var(--text);
  white-space: pre-wrap;
  word-break: break-word;
  overflow: auto;
}

.diff-header,
.diff-row {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr) minmax(0, 1fr);
}

.diff-header {
  padding: 0 0 10px;
  color: var(--text-mute);
  font-size: 0.82rem;
  text-transform: uppercase;
  letter-spacing: 0.06em;
}

.diff-table {
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
}

.source-editor-shell {
  height: 420px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  background: var(--panel-strong);
}

.source-editor-shell :deep(.code-editor-shell) {
  height: 420px;
}

.source-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.diff-row {
  border-top: 1px solid var(--border);
}

.diff-row:first-child {
  border-top: 0;
}

.diff-line,
.diff-cell {
  padding: 12px 14px;
}

.diff-line {
  color: var(--text-mute);
  background: var(--panel-muted);
  border-right: 1px solid var(--border);
}

.diff-cell {
  display: block;
  min-height: 100%;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: var(--font-code);
}

.diff-row-same .diff-cell {
  background: var(--panel-strong);
}

.diff-row-changed .diff-cell {
  background: color-mix(in srgb, var(--warning) 12%, transparent);
}

.diff-row-missing .diff-cell:first-of-type {
  background: color-mix(in srgb, var(--warning) 14%, transparent);
}

.diff-row-missing .diff-cell:last-of-type {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

.diff-row-extra .diff-cell:first-of-type {
  background: color-mix(in srgb, var(--danger) 10%, transparent);
}

.diff-row-extra .diff-cell:last-of-type {
  background: color-mix(in srgb, var(--warning) 14%, transparent);
}

.char-changed {
  background: color-mix(in srgb, var(--danger) 22%, transparent);
  border-radius: 6px;
  padding: 1px 2px;
}
</style>
