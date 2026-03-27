<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink, useRoute } from 'vue-router'

import { api, type ApiProblem } from '../lib/api'
import { renderMarkdown } from '../lib/markdown'

const route = useRoute()
const problem = ref<ApiProblem | null>(null)
const error = ref('')

onMounted(async () => {
  try {
    const response = await api.getProblem(String(route.params.problemId))
    problem.value = response.problem
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载题目失败'
  }
})

const currentProblem = computed(() => problem.value)
const statementHtml = computed(() =>
  renderMarkdown(currentProblem.value?.statement_md || currentProblem.value?.description || '暂无题面'),
)
const inputHtml = computed(() =>
  renderMarkdown(currentProblem.value?.input_desc_md || '输入格式以题面与样例为准。'),
)
const outputHtml = computed(() =>
  renderMarkdown(currentProblem.value?.output_desc_md || '输出需与标准答案严格匹配。'),
)
const resourceLimitRows = computed(() =>
  Object.entries(currentProblem.value?.resource_limits ?? {}).map(([language, limits]) => ({
    language: language === 'cpp' ? 'C++' : language === 'python' ? 'Python 3' : language,
    time: limits.time_limit_ms % 1000 === 0 ? `${limits.time_limit_ms / 1000}s` : `${limits.time_limit_ms}ms`,
    memory:
      limits.memory_limit_kb % 1024 === 0 ? `${limits.memory_limit_kb / 1024}MB` : `${limits.memory_limit_kb}KB`,
  })),
)
const easyMeta = computed(() => (currentProblem.value?.easy_judger ?? {}) as Record<string, unknown>)
const easyOptionRows = computed(() => {
  const metadata = (easyMeta.value.metadata ?? {}) as Record<string, unknown>
  const descriptions = (metadata.option_descriptions ?? {}) as Record<string, unknown>
  const options = Array.isArray(metadata.options) ? metadata.options : []
  return options.map((option) => ({
    option: String(option),
    description: String(descriptions[String(option)] ?? ''),
  }))
})
</script>

<template>
  <div class="page">
    <p v-if="error" class="muted">{{ error }}</p>
    <div class="page-header">
      <div>
        <span class="page-kicker">{{ currentProblem?.judge_mode?.toUpperCase() ?? 'PROBLEM' }}</span>
        <h2 class="page-title">{{ currentProblem?.title ?? '题目详情' }}</h2>
        <p class="page-subtitle">
          这里展示的是题目详情页原型。正式接入后可以对接题面、样例、限制、函数签名和 EasyJudger 元数据。
        </p>
      </div>
      <RouterLink v-if="currentProblem" class="action-button" :to="`/problems/${currentProblem.problem_id}/submit`">开始提交</RouterLink>
    </div>

    <section class="section-grid">
      <div class="span-8 detail-card">
        <div class="meta-row">
          <span class="tag">{{ currentProblem?.judge_mode ?? '--' }}</span>
          <span class="tag">{{ (currentProblem?.languages ?? []).join(' / ') || '--' }}</span>
        </div>

        <h3 class="section-title">题目描述</h3>
        <div class="markdown-body" v-html="statementHtml"></div>

        <template v-if="currentProblem?.judge_mode !== 'easy'">
          <h3 class="section-title">输入说明</h3>
          <div class="markdown-body" v-html="inputHtml"></div>

          <h3 class="section-title">输出说明</h3>
          <div class="markdown-body" v-html="outputHtml"></div>
        </template>

        <template v-else>
          <h3 class="section-title">作答方式</h3>
          <div class="easy-card">
            <p class="muted">题型：{{ String(easyMeta.question_type ?? 'easy') }}</p>
            <div v-if="easyOptionRows.length" class="easy-options">
              <div v-for="row in easyOptionRows" :key="row.option" class="easy-option-row">
                <strong>{{ row.option }}.</strong>
                <span>{{ row.description || '未填写选项说明' }}</span>
              </div>
            </div>
            <p v-else class="muted">判断题将在作答页直接显示“正确 / 错误”切换。</p>
          </div>
        </template>
      </div>

      <div class="span-4 detail-card">
        <div v-if="currentProblem?.judge_mode !== 'easy'">
          <span class="eyebrow">样例与限制</span>
          <div v-if="resourceLimitRows.length" class="limit-list">
            <div v-for="row in resourceLimitRows" :key="row.language" class="limit-row">
              <strong>{{ row.language }}</strong>
              <span>{{ row.time }} / {{ row.memory }}</span>
            </div>
          </div>
          <pre class="editor-block"><code>正式样例将直接读取题目 testcases / samples_json。</code></pre>
        </div>
        <div v-else>
          <span class="eyebrow">Easy 信息</span>
          <div class="limit-list">
            <div class="limit-row">
              <strong>题型</strong>
              <span>{{ String(easyMeta.question_type ?? '--') }}</span>
            </div>
            <div class="limit-row">
              <strong>满分</strong>
              <span>{{ String(((easyMeta.metadata ?? {}) as Record<string, unknown>).full_score ?? 1) }}</span>
            </div>
          </div>
        </div>
        <div class="inline-actions">
          <RouterLink v-if="currentProblem" class="action-button" :to="`/problems/${currentProblem.problem_id}/submit`">
            {{ currentProblem.judge_mode === 'easy' ? '开始作答' : '去提交' }}
          </RouterLink>
          <RouterLink class="ghost-button" to="/submissions">看记录</RouterLink>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.markdown-body {
  color: var(--text-soft);
}

.markdown-body :deep(p),
.markdown-body :deep(ul),
.markdown-body :deep(ol) {
  margin: 0 0 12px;
}

.markdown-body :deep(code) {
  padding: 0.12rem 0.38rem;
  border-radius: 8px;
  background: var(--panel-strong);
  font-family: var(--font-code);
}

.markdown-body :deep(pre) {
  margin: 0 0 12px;
  padding: 14px 16px;
  border-radius: var(--radius-md);
  background: var(--panel-strong);
  overflow: auto;
}

.markdown-body :deep(pre code) {
  padding: 0;
  background: transparent;
}

.limit-list,
.easy-options {
  display: grid;
  gap: 10px;
}

.limit-row,
.easy-option-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
}

.easy-card {
  display: grid;
  gap: 12px;
}
</style>
