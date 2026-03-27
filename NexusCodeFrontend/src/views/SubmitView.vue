<script setup lang="ts">
import { defineAsyncComponent, computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { supportedLanguages } from '../data/mock'
import { api, type ApiProblem } from '../lib/api'
import { renderMarkdown } from '../lib/markdown'

const CodeEditor = defineAsyncComponent(() => import('../components/CodeEditor.vue'))
const EDITOR_FONT_KEY = 'nexusoj.editor.font_size'
const EDITOR_LINE_NUMBERS_KEY = 'nexusoj.editor.line_numbers'
const EDITOR_TAB_SIZE_KEY = 'nexusoj.editor.tab_size'
const EDITOR_DRAFT_PREFIX = 'nexusoj.editor.draft'

const route = useRoute()
const router = useRouter()
const problem = ref<ApiProblem | null>(null)
const submitError = ref('')
const submitting = ref(false)
const languageMenuOpen = ref(false)
const languageMenuRef = ref<HTMLElement | null>(null)

const templates = {
  cpp: `#include <bits/stdc++.h>
using namespace std;

int main() {
  ios::sync_with_stdio(false);
  cin.tie(nullptr);

  return 0;
}
`,
  python: `import sys


def solve() -> None:
    pass


if __name__ == "__main__":
    solve()
`,
} satisfies Record<'cpp' | 'python', string>

const functionalDefaultReturn = (returnType: string, language: 'cpp' | 'python') => {
  const normalized = returnType.trim().toLowerCase()
  if (!normalized || normalized === 'void' || normalized === 'none') return ''
  if (language === 'cpp') {
    if (normalized === 'bool') return '  return false;\n'
    if (normalized === 'string' || normalized === 'std::string') return '  return "";\n'
    if (normalized.includes('vector')) return '  return {};\n'
    return '  return {};\n'
  }
  if (normalized === 'bool') return '    return False\n'
  if (normalized === 'str' || normalized === 'string') return '    return ""\n'
  if (normalized.startsWith('list') || normalized.startsWith('tuple')) return '    return []\n'
  return '    return None\n'
}

const buildFunctionalTemplate = (language: 'cpp' | 'python', problemValue: ApiProblem | null) => {
  const details = problemValue?.function_details_json as
    | {
        function_name?: string
        return_type?: string
        params?: Array<{ name?: string; type?: string }>
        arguments?: Array<{ name?: string; type?: string }>
      }
    | undefined
  if (!details || typeof details !== 'object') {
    return templates[language]
  }

  const functionName = details.function_name?.trim() || 'solve'
  const returnType = details.return_type?.trim() || (language === 'cpp' ? 'int' : 'None')
  const argumentsList = Array.isArray(details.params)
    ? details.params
    : Array.isArray(details.arguments)
      ? details.arguments
      : []

  if (language === 'cpp') {
    const signatureArgs = argumentsList
      .map((argument) => `${argument.type?.trim() || 'int'} ${argument.name?.trim() || 'arg'}`)
      .join(', ')
    return `#include <bits/stdc++.h>
using namespace std;

class Solution {
public:
  ${returnType} ${functionName}(${signatureArgs}) {
${functionalDefaultReturn(returnType, 'cpp')}  }
};
`
  }

  const signatureArgs = argumentsList.map((argument) => argument.name?.trim() || 'arg').join(', ')
  return `def ${functionName}(${signatureArgs}) -> ${returnType || 'None'}:
${functionalDefaultReturn(returnType, 'python')}
`
}

const getDefaultTemplate = (language: 'cpp' | 'python', problemValue: ApiProblem | null) =>
  problemValue?.judge_mode === 'functional' ? buildFunctionalTemplate(language, problemValue) : templates[language]

const selectedLanguage = ref<'cpp' | 'python'>('cpp')
const code = ref(templates.cpp)
const editorFontSize = ref(15)
const showLineNumbers = ref(true)
const editorTabSize = ref(4)
const easySingleAnswer = ref('A')
const easyMultipleAnswer = ref<string[]>([])
const easyTrueFalseAnswer = ref<'TRUE' | 'FALSE'>('TRUE')

const handleGlobalPointer = (event: MouseEvent) => {
  if (!languageMenuRef.value) return
  const target = event.target
  if (target instanceof Node && !languageMenuRef.value.contains(target)) {
    languageMenuOpen.value = false
  }
}

onMounted(async () => {
  document.addEventListener('click', handleGlobalPointer)

  const storedFontSize = Number(localStorage.getItem(EDITOR_FONT_KEY) ?? '')
  if (!Number.isNaN(storedFontSize) && storedFontSize >= 12 && storedFontSize <= 22) {
    editorFontSize.value = storedFontSize
  }

  const storedLineNumbers = localStorage.getItem(EDITOR_LINE_NUMBERS_KEY)
  if (storedLineNumbers === 'false') {
    showLineNumbers.value = false
  }

  const storedTabSize = Number(localStorage.getItem(EDITOR_TAB_SIZE_KEY) ?? '')
  if (!Number.isNaN(storedTabSize) && [2, 4, 8].includes(storedTabSize)) {
    editorTabSize.value = storedTabSize
  }

  try {
    const response = await api.getProblem(String(route.params.problemId))
    problem.value = response.problem
    if (response.problem.judge_mode === 'easy') {
      const metadata = ((response.problem.easy_judger ?? {}) as Record<string, unknown>).metadata as
        | Record<string, unknown>
        | undefined
      const options = Array.isArray(metadata?.options) ? metadata.options.map((item) => String(item)) : ['A', 'B', 'C', 'D']
      easySingleAnswer.value = options[0] ?? 'A'
      easyMultipleAnswer.value = []
      easyTrueFalseAnswer.value = 'TRUE'
      return
    }
    const allowed = response.problem.languages ?? []
    if (allowed.length > 0 && !allowed.includes(selectedLanguage.value)) {
      selectedLanguage.value = allowed.includes('python') ? 'python' : 'cpp'
    }

    code.value = loadDraft(selectedLanguage.value)
  } catch (err) {
    submitError.value = err instanceof Error ? err.message : '加载题目失败'
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('click', handleGlobalPointer)
})

const draftStorageKey = (language: 'cpp' | 'python') =>
  `${EDITOR_DRAFT_PREFIX}:${String(route.params.problemId)}:${language}`

const loadDraft = (language: 'cpp' | 'python') =>
  localStorage.getItem(draftStorageKey(language)) ?? getDefaultTemplate(language, problem.value)

const saveDraft = (language: 'cpp' | 'python', value: string) => {
  localStorage.setItem(draftStorageKey(language), value)
}

const setLanguage = (language: 'cpp' | 'python') => {
  saveDraft(selectedLanguage.value, code.value)
  selectedLanguage.value = language
  code.value = loadDraft(language)
  languageMenuOpen.value = false
}

const availableLanguages = computed(() =>
  supportedLanguages.filter((language) => (problem.value?.languages ?? []).includes(language.value)),
)

const selectedLanguageLabel = computed(
  () => availableLanguages.value.find((item) => item.value === selectedLanguage.value)?.label ?? '选择语言',
)

const toggleLanguageMenu = () => {
  languageMenuOpen.value = !languageMenuOpen.value
}

const decreaseFontSize = () => {
  editorFontSize.value = Math.max(12, editorFontSize.value - 1)
  localStorage.setItem(EDITOR_FONT_KEY, String(editorFontSize.value))
}

const increaseFontSize = () => {
  editorFontSize.value = Math.min(22, editorFontSize.value + 1)
  localStorage.setItem(EDITOR_FONT_KEY, String(editorFontSize.value))
}

const toggleLineNumbers = () => {
  showLineNumbers.value = !showLineNumbers.value
  localStorage.setItem(EDITOR_LINE_NUMBERS_KEY, String(showLineNumbers.value))
}

const setTabSize = (value: number) => {
  editorTabSize.value = value
  localStorage.setItem(EDITOR_TAB_SIZE_KEY, String(value))
}

watch(code, (nextCode) => {
  saveDraft(selectedLanguage.value, nextCode)
})

const submit = async () => {
  if (!problem.value) return
  try {
    submitting.value = true
    submitError.value = ''
    let submissionId = ''
    if (problem.value.judge_mode === 'easy') {
      const response = await api.createEasySubmission({
        problem_id: problem.value.problem_id,
        answer:
          easyQuestionType.value === 'multiple_choice'
            ? easyMultipleAnswer.value
            : easyQuestionType.value === 'true_false'
              ? easyTrueFalseAnswer.value
              : easySingleAnswer.value,
      })
      submissionId = response.submission_id
    } else {
      const response = await api.createSubmission({
        problem_id: problem.value.problem_id,
        language: selectedLanguage.value,
        source_code: code.value,
      })
      submissionId = response.submission_id
    }
    await router.push(`/submissions/${submissionId}`)
  } catch (err) {
    submitError.value = err instanceof Error ? err.message : '提交失败'
  } finally {
    submitting.value = false
  }
}

const statementHtml = computed(() =>
  renderMarkdown(problem.value?.statement_md || problem.value?.description || '暂无题目描述'),
)

const inputHtml = computed(() =>
  renderMarkdown(
    problem.value?.input_desc_md ||
      (problem.value?.judge_mode === 'functional'
        ? '请按照函数签名实现目标函数，参数由评测系统自动注入。'
        : '输入格式以题面与样例为准。'),
  ),
)

const outputHtml = computed(() =>
  renderMarkdown(
    problem.value?.output_desc_md ||
      (problem.value?.judge_mode === 'easy'
        ? '根据题目标准答案完成选择或判断。'
        : '输出需与标准答案严格匹配。'),
  ),
)

const sampleEntries = computed(() => {
  const rawSamples =
    problem.value?.samples_json?.length
      ? problem.value.samples_json
      : (problem.value?.testcases?.slice(0, 2) ?? [])

  return rawSamples
    .map((sample, index) => {
      const input = String(sample.input ?? sample.input_data ?? '').trim()
      const output = String(sample.output ?? sample.expected_output ?? '').trim()
      if (!input && !output) return null
      return {
        title: `样例 ${index + 1}`,
        input: input || '无',
        output: output || '无',
      }
    })
    .filter((item): item is { title: string; input: string; output: string } => Boolean(item))
})

const easyMeta = computed(() => (problem.value?.easy_judger ?? {}) as Record<string, unknown>)
const easyQuestionType = computed(() => String(easyMeta.value.question_type ?? 'single_choice'))
const easyOptionRows = computed(() => {
  const metadata = (easyMeta.value.metadata ?? {}) as Record<string, unknown>
  const descriptions = (metadata.option_descriptions ?? {}) as Record<string, unknown>
  const options = Array.isArray(metadata.options) ? metadata.options : ['A', 'B', 'C', 'D']
  return options.map((option) => ({
    option: String(option),
    description: String(descriptions[String(option)] ?? ''),
  }))
})

</script>

<template>
  <div class="page submit-page">
    <p v-if="submitError" class="muted">{{ submitError }}</p>

    <div class="submit-topbar">
      <div>
        <span class="page-kicker">Submit</span>
        <h2 class="page-title">{{ problem?.title ?? route.params.problemId }}</h2>
      </div>
      <div class="toolbar">
        <RouterLink class="ghost-button" :to="`/problems/${route.params.problemId}`">返回题面</RouterLink>
        <button class="action-button" type="button" :disabled="submitting" @click="submit">
          {{ submitting ? '提交中...' : '提交评测' }}
        </button>
      </div>
    </div>

    <section class="leetcode-workspace">
      <article class="leetcode-pane problem-pane">
        <div class="leetcode-pane-header">
          <span class="eyebrow">{{ problem?.judge_mode?.toUpperCase() ?? 'PROBLEM' }}</span>
          <div class="tag-row">
            <span class="tag" v-for="tag in problem?.tags ?? []" :key="tag">{{ tag }}</span>
          </div>
        </div>

        <div class="leetcode-pane-body problem-body">
          <section class="statement-section">
            <h3>题目描述</h3>
            <div class="markdown-body" v-html="statementHtml"></div>
          </section>

          <section v-if="problem?.judge_mode !== 'easy'" class="statement-section">
            <h3>输入说明</h3>
            <div class="markdown-body" v-html="inputHtml"></div>
          </section>

          <section v-if="problem?.judge_mode !== 'easy'" class="statement-section">
            <h3>输出说明</h3>
            <div class="markdown-body" v-html="outputHtml"></div>
          </section>

          <section v-if="problem?.judge_mode === 'easy'" class="statement-section">
            <h3>作答说明</h3>
            <div class="easy-submit-summary">
              <p class="muted">题型：{{ easyQuestionType === 'true_false' ? '判断题' : easyQuestionType === 'single_choice' ? '单选题' : '多选题' }}</p>
              <div v-if="easyOptionRows.length" class="easy-submit-options">
                <div v-for="row in easyOptionRows" :key="row.option" class="easy-submit-option-row">
                  <strong>{{ row.option }}.</strong>
                  <span>{{ row.description || '未填写选项说明' }}</span>
                </div>
              </div>
              <p v-else class="muted">请在右侧直接选择“正确”或“错误”。</p>
            </div>
          </section>

          <section v-if="problem?.judge_mode !== 'easy' && sampleEntries.length" class="statement-section">
            <h3>样例</h3>
            <div class="sample-stack">
              <div v-for="sample in sampleEntries" :key="sample.title" class="sample-item">
                <span class="eyebrow">{{ sample.title }}</span>
                <pre class="sample-block"><code>输入:
{{ sample.input }}

输出:
{{ sample.output }}</code></pre>
              </div>
            </div>
          </section>
        </div>
      </article>

      <article class="leetcode-pane editor-pane">
        <div class="leetcode-pane-header editor-toolbar">
          <div v-if="problem?.judge_mode !== 'easy'" class="editor-controls">
            <label class="language-select-shell">
              <span class="eyebrow">Language</span>
              <div ref="languageMenuRef" class="language-menu-shell">
                <button
                  class="language-trigger"
                  :class="{ 'language-trigger-open': languageMenuOpen }"
                  type="button"
                  aria-haspopup="listbox"
                  :aria-expanded="languageMenuOpen ? 'true' : 'false'"
                  title="选择编程语言"
                  @click="toggleLanguageMenu"
                >
                  <span class="language-trigger-label">{{ selectedLanguageLabel }}</span>
                  <span class="language-trigger-icon"></span>
                </button>

                <div v-if="languageMenuOpen" class="language-menu" role="listbox" aria-label="编程语言">
                  <button
                    v-for="language in availableLanguages"
                    :key="language.value"
                    class="language-option"
                    :class="{ 'language-option-active': selectedLanguage === language.value }"
                    type="button"
                    role="option"
                    :aria-selected="selectedLanguage === language.value ? 'true' : 'false'"
                    @click="setLanguage(language.value)"
                  >
                    <span>{{ language.label }}</span>
                    <span v-if="selectedLanguage === language.value" class="language-option-check">✓</span>
                  </button>
                </div>
              </div>
            </label>

            <div class="font-controls">
              <div class="font-actions">
                <button
                  class="font-button"
                  type="button"
                  title="减小编辑器字号"
                  aria-label="减小编辑器字号"
                  @click="decreaseFontSize"
                >
                  A-
                </button>
                <button
                  class="font-button"
                  type="button"
                  title="增大编辑器字号"
                  aria-label="增大编辑器字号"
                  @click="increaseFontSize"
                >
                  A+
                </button>
                <button
                  class="font-button"
                  :class="{ 'font-button-active': showLineNumbers }"
                  type="button"
                  title="显示或隐藏行号"
                  aria-label="显示或隐藏行号"
                  @click="toggleLineNumbers"
                >
                  Ln
                </button>
                <label class="tab-select-shell" title="设置 Tab 缩进空格数">
                  <span class="sr-only">Tab 缩进空格数</span>
                  <div class="select-shell select-shell-compact">
                    <select
                      class="tab-select"
                      :value="editorTabSize"
                      @change="setTabSize(Number(($event.target as HTMLSelectElement).value))"
                    >
                      <option :value="2">Tab 2</option>
                      <option :value="4">Tab 4</option>
                      <option :value="8">Tab 8</option>
                    </select>
                  </div>
                </label>
              </div>
            </div>
          </div>
          <div v-else class="easy-answer-header">
            <div>
              <span class="eyebrow">Easy Answer</span>
              <strong>请选择你的答案</strong>
            </div>
          </div>
        </div>

        <div v-if="problem?.judge_mode !== 'easy'" class="leetcode-pane-body editor-body">
          <CodeEditor
            v-model="code"
            :language="selectedLanguage"
            :font-size="editorFontSize"
            :line-numbers="showLineNumbers"
            :tab-size="editorTabSize"
          />
        </div>
        <div v-else class="leetcode-pane-body easy-answer-body">
          <div class="easy-answer-card">
            <template v-if="easyQuestionType === 'true_false'">
              <div class="easy-boolean-toggle">
                <button
                  class="easy-option-button"
                  :class="{ 'easy-option-active': easyTrueFalseAnswer === 'TRUE' }"
                  type="button"
                  @click="easyTrueFalseAnswer = 'TRUE'"
                >
                  正确
                </button>
                <button
                  class="easy-option-button"
                  :class="{ 'easy-option-active': easyTrueFalseAnswer === 'FALSE' }"
                  type="button"
                  @click="easyTrueFalseAnswer = 'FALSE'"
                >
                  错误
                </button>
              </div>
            </template>

            <template v-else-if="easyQuestionType === 'single_choice'">
              <button
                v-for="row in easyOptionRows"
                :key="row.option"
                class="easy-option-button easy-option-row"
                :class="{ 'easy-option-active': easySingleAnswer === row.option }"
                type="button"
                @click="easySingleAnswer = row.option"
              >
                <strong>{{ row.option }}.</strong>
                <span>{{ row.description || '未填写选项说明' }}</span>
              </button>
            </template>

            <template v-else>
              <button
                v-for="row in easyOptionRows"
                :key="row.option"
                class="easy-option-button easy-option-row"
                :class="{ 'easy-option-active': easyMultipleAnswer.includes(row.option) }"
                type="button"
                @click="
                  easyMultipleAnswer = easyMultipleAnswer.includes(row.option)
                    ? easyMultipleAnswer.filter((item) => item !== row.option)
                    : [...easyMultipleAnswer, row.option]
                "
              >
                <strong>{{ row.option }}.</strong>
                <span>{{ row.description || '未填写选项说明' }}</span>
              </button>
            </template>
          </div>
        </div>
      </article>
    </section>
  </div>
</template>

<style scoped>
.submit-page {
  gap: 12px;
  min-height: calc(100vh - 140px);
}

.submit-topbar {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 18px;
}

.leetcode-workspace {
  display: grid;
  grid-template-columns: minmax(360px, 0.92fr) minmax(620px, 1.08fr);
  gap: 16px;
  flex: 1;
  min-height: 0;
  height: calc(100vh - 170px);
  max-height: calc(100vh - 170px);
}

.leetcode-pane {
  min-height: 0;
  display: grid;
  grid-template-rows: auto minmax(0, 1fr);
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow);
  overflow: hidden;
}

.leetcode-pane-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border);
  background: var(--panel-strong);
}

.leetcode-pane-body {
  min-height: 0;
  height: 100%;
}

.problem-body {
  padding: 22px;
  overflow: auto;
  min-height: 0;
  overscroll-behavior: contain;
}

.statement-section + .statement-section {
  margin-top: 24px;
}

.statement-section h3 {
  margin: 0 0 10px;
  font-family: var(--font-heading);
  font-size: 1rem;
}

.sample-block {
  margin: 0;
  padding: 16px 18px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
  overflow: auto;
}

.sample-stack {
  display: grid;
  gap: 14px;
}

.sample-item {
  display: grid;
  gap: 8px;
}

.easy-submit-summary,
.easy-submit-options {
  display: grid;
  gap: 12px;
}

.easy-submit-option-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 12px 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
}

.editor-toolbar {
  justify-content: flex-start;
}

.editor-controls {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 18px;
  width: 100%;
}

.easy-answer-header {
  display: flex;
  align-items: center;
}

.easy-answer-body {
  padding: 20px;
  overflow: auto;
}

.easy-answer-card {
  display: grid;
  gap: 14px;
}

.easy-boolean-toggle {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.easy-option-button {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 18px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
  color: var(--text);
  text-align: left;
}

.easy-option-row {
  justify-content: flex-start;
}

.easy-option-active {
  border-color: var(--accent);
  background: var(--accent-soft);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 28%, transparent);
}

.language-select-shell {
  display: flex;
  align-items: flex-start;
  flex-direction: column;
  gap: 8px;
  min-width: 220px;
}

.language-menu-shell {
  position: relative;
  display: inline-flex;
  min-width: 220px;
}

.language-trigger {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  min-height: 48px;
  padding: 11px 14px 11px 16px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background:
    linear-gradient(180deg, color-mix(in srgb, var(--panel-strong) 94%, white), var(--panel-strong));
  color: var(--text);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.28),
    0 8px 24px rgba(15, 23, 42, 0.06);
  transition:
    border-color 180ms ease,
    background-color 180ms ease,
    box-shadow 180ms ease,
    transform 180ms ease;
}

.language-trigger:hover,
.language-trigger-open {
  border-color: var(--border-strong);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.3),
    0 12px 28px rgba(15, 23, 42, 0.1);
}

.language-trigger:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 56%, transparent);
  outline-offset: 2px;
}

.language-trigger-label {
  font-weight: 700;
  letter-spacing: 0.01em;
}

.language-trigger-icon {
  width: 10px;
  height: 10px;
  border-right: 2px solid var(--text-mute);
  border-bottom: 2px solid var(--text-mute);
  transform: translateY(-2px) rotate(45deg);
  transition: transform 180ms ease;
}

.language-trigger-open .language-trigger-icon {
  transform: translateY(1px) rotate(-135deg);
}

.language-menu {
  position: absolute;
  top: calc(100% + 10px);
  left: 0;
  z-index: 20;
  display: grid;
  gap: 6px;
  min-width: 220px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 16px;
  background:
    linear-gradient(180deg, color-mix(in srgb, var(--panel-strong) 96%, white), var(--panel));
  box-shadow:
    0 20px 48px rgba(15, 23, 42, 0.16),
    inset 0 1px 0 rgba(255, 255, 255, 0.18);
  backdrop-filter: blur(18px);
}

.language-option {
  display: inline-flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  min-height: 42px;
  padding: 10px 12px;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: var(--text);
  font-weight: 600;
  text-align: left;
  transition:
    background-color 180ms ease,
    color 180ms ease;
}

.language-option:hover {
  background: var(--accent-soft);
}

.language-option-active {
  background: color-mix(in srgb, var(--accent-soft) 88%, transparent);
  color: var(--accent-strong);
}

.language-option-check {
  font-size: 0.92rem;
  font-weight: 800;
}

.font-controls {
  display: flex;
  align-items: center;
}

.font-actions {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: 14px;
  background: var(--panel-strong);
}

.font-button {
  border: 0;
  background: transparent;
  color: var(--text);
  border-radius: 10px;
  min-width: 40px;
  padding: 9px 10px;
  font-weight: 700;
}

.font-button:hover {
  background: var(--accent-soft);
}

.font-button-active {
  background: var(--accent-soft);
  color: var(--accent-strong);
}

.tab-select-shell {
  display: inline-flex;
}

.tab-select {
  min-width: 72px;
  border: 0;
  padding: 9px 30px 9px 10px;
  background: transparent;
  color: var(--text);
  font-weight: 700;
  appearance: none;
  -webkit-appearance: none;
}

.select-shell-compact {
  min-width: 88px;
  border-radius: 10px;
}

.select-shell-compact::after {
  right: 10px;
  width: 8px;
  height: 8px;
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.editor-body {
  padding: 0;
  min-height: 0;
  overflow: hidden;
  height: 100%;
}

.editor-body :deep(.code-editor-shell) {
  height: 100%;
  min-height: 100%;
  border-radius: 0;
}

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

@media (max-width: 1180px) {
  .leetcode-workspace {
    grid-template-columns: 1fr;
    height: auto;
    max-height: none;
  }

  .submit-topbar {
    align-items: flex-start;
    flex-direction: column;
  }

  .problem-body,
  .editor-body {
    height: min(60vh, 720px);
    max-height: min(60vh, 720px);
  }

  .editor-controls {
    align-items: stretch;
    flex-direction: column;
  }

  .font-controls {
    align-items: flex-start;
  }
}
</style>
