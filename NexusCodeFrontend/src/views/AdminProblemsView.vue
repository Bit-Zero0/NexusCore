<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api, type ApiProblem } from '../lib/api'

const problems = ref<ApiProblem[]>([])
const loading = ref(false)
const error = ref('')
const importDialogOpen = ref(false)
const importText = ref('[\n  {\n    "problem_id": "sample-import-1",\n    "title": "导入示例题",\n    "judge_mode": "acm",\n    "languages": ["cpp", "python"],\n    "resource_limits": {\n      "cpp": { "time_limit_ms": 1000, "memory_limit_kb": 51200 },\n      "python": { "time_limit_ms": 2000, "memory_limit_kb": 102400 }\n    },\n    "testcases": [\n      { "input": "1 2\\n", "output": "3\\n" }\n    ]\n  }\n]')
const importLoading = ref(false)
const notices = ref<Array<{ id: string; type: 'success' | 'error'; message: string }>>([])

const pushNotice = (type: 'success' | 'error', message: string) => {
  const id = crypto.randomUUID()
  notices.value.push({ id, type, message })
  window.setTimeout(() => {
    notices.value = notices.value.filter((notice) => notice.id !== id)
  }, 3600)
}

const loadProblems = async () => {
  try {
    loading.value = true
    error.value = ''
    const response = await api.listProblems()
    problems.value = response.problems
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载题目列表失败'
  } finally {
    loading.value = false
  }
}

onMounted(async () => {
  await loadProblems()
})

const parseImportPayload = (raw: string) => {
  const parsed = JSON.parse(raw) as unknown
  if (Array.isArray(parsed)) {
    return parsed as Array<Record<string, unknown>>
  }
  if (parsed && typeof parsed === 'object' && Array.isArray((parsed as { problems?: unknown }).problems)) {
    return (parsed as { problems: Array<Record<string, unknown>> }).problems
  }
  throw new Error('导入内容必须是题目数组，或形如 { "problems": [...] } 的对象')
}

const handleImportFile = async (event: Event) => {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return
  importText.value = await file.text()
  input.value = ''
}

const importProblems = async () => {
  try {
    importLoading.value = true
    const items = parseImportPayload(importText.value)
    if (items.length === 0) {
      throw new Error('导入列表不能为空')
    }

    let successCount = 0
    for (const item of items) {
      await api.createProblem(item)
      successCount += 1
    }

    pushNotice('success', `成功导入 ${successCount} 道题目`)
    importDialogOpen.value = false
    await loadProblems()
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '多题导入失败')
  } finally {
    importLoading.value = false
  }
}
</script>

<template>
  <div class="page">
    <div class="page-notices">
      <div v-for="notice in notices" :key="notice.id" class="page-notice" :class="`page-notice-${notice.type}`">
        {{ notice.message }}
      </div>
    </div>

    <div class="page-header">
      <div>
        <span class="page-kicker">Admin / Problems</span>
        <h2 class="page-title">录题管理</h2>
        <p class="page-subtitle">现在这里已经直接读取当前 OJ 题库接口，可继续在这里做题目维护与录题流转。</p>
      </div>
      <div class="toolbar">
        <button class="ghost-button" type="button" :disabled="loading" @click="loadProblems">
          {{ loading ? '刷新中...' : '刷新列表' }}
        </button>
        <button class="ghost-button" type="button" @click="importDialogOpen = true">多题导入</button>
        <RouterLink class="action-button" to="/admin/problems/new">新建题目</RouterLink>
      </div>
    </div>

    <p v-if="error" class="muted">{{ error }}</p>

    <section class="table-shell">
      <table>
        <thead>
          <tr>
            <th>题目</th>
            <th>模式</th>
            <th>语言</th>
            <th>标签</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="!loading && problems.length === 0">
            <td colspan="5" class="muted empty-cell">暂无题目</td>
          </tr>
          <tr v-for="problem in problems" :key="problem.problem_id">
            <td>
              <strong>{{ problem.title }}</strong>
              <div class="muted">{{ problem.problem_id }}</div>
            </td>
            <td><span class="tag">{{ problem.judge_mode }}</span></td>
            <td>{{ (problem.languages ?? []).join(' / ') || '--' }}</td>
            <td>{{ (problem.tags ?? []).join(' / ') || '--' }}</td>
            <td>
              <div class="inline-actions">
                <RouterLink class="ghost-button" :to="`/admin/problems/${problem.problem_id}/edit`">编辑</RouterLink>
                <RouterLink class="ghost-button" :to="`/problems/${problem.problem_id}`">预览</RouterLink>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </section>

    <div v-if="importDialogOpen" class="dialog-backdrop" @click.self="importDialogOpen = false">
      <section class="dialog-card">
        <div class="page-header">
          <div>
            <span class="page-kicker">Batch Import</span>
            <h3 class="page-title">多题导入</h3>
            <p class="page-subtitle">支持直接复制粘贴 JSON，或选择本地 JSON 文件。导入内容可以是数组，也可以是 `{ "problems": [...] }`。</p>
          </div>
          <div class="toolbar">
            <label class="ghost-button file-button">
              选择 JSON 文件
              <input type="file" accept=".json,application/json" @change="handleImportFile" />
            </label>
            <button class="ghost-button" type="button" @click="importDialogOpen = false">关闭</button>
          </div>
        </div>

        <textarea v-model="importText" class="import-textarea" spellcheck="false"></textarea>

        <div class="toolbar">
          <button class="action-button" type="button" :disabled="importLoading" @click="importProblems">
            {{ importLoading ? '导入中...' : '开始导入' }}
          </button>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.empty-cell {
  text-align: center;
}

.page-notices {
  position: fixed;
  top: 92px;
  right: 28px;
  z-index: 40;
  display: grid;
  gap: 10px;
}

.page-notice {
  min-width: 240px;
  max-width: 320px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--panel-strong);
  box-shadow: var(--shadow);
}

.page-notice-success {
  border-color: color-mix(in srgb, var(--success) 34%, var(--border));
}

.page-notice-error {
  border-color: color-mix(in srgb, var(--danger) 34%, var(--border));
}

.dialog-backdrop {
  position: fixed;
  inset: 0;
  z-index: 35;
  display: grid;
  place-items: center;
  padding: 24px;
  background: rgba(12, 14, 20, 0.48);
}

.dialog-card {
  width: min(980px, 100%);
  max-height: min(88vh, 920px);
  display: grid;
  gap: 16px;
  padding: 22px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--panel);
  box-shadow: var(--shadow);
}

.import-textarea {
  min-height: 420px;
  max-height: 56vh;
  resize: vertical;
  font-family: var(--font-code);
}

.file-button {
  position: relative;
  overflow: hidden;
}

.file-button input {
  position: absolute;
  inset: 0;
  opacity: 0;
  cursor: pointer;
}
</style>
