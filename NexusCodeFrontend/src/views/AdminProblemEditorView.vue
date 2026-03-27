<script setup lang="ts">
import { defineAsyncComponent, computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, type ApiProblem } from '../lib/api'

const CodeEditor = defineAsyncComponent(() => import('../components/CodeEditor.vue'))

type JudgeMode = 'acm' | 'functional' | 'easy'
type TestcaseRow = {
  id: string
  input: string
  output: string
}
type FunctionArgumentRow = {
  id: string
  name: string
  type: string
}
type ResourceLimitForm = {
  timeValue: string
  timeUnit: 'ms' | 's'
  memoryValue: string
  memoryUnit: 'kb' | 'mb'
}
type EasyOptionRow = {
  id: string
  key: string
  description: string
}

const route = useRoute()
const router = useRouter()
const isNew = computed(() => route.name === 'admin-problems-new')

const problemId = ref('')
const title = ref('')
const description = ref('')
const judgeMode = ref<JudgeMode>('acm')
const judgeQueue = ref<'' | 'fast' | 'normal' | 'heavy'>('')
const judgeMethod = ref<'validator' | 'spj'>('validator')
const sandboxKind = ref<'nsjail' | 'wasm' | 'nsjail_wasm'>('nsjail')
const validatorMode = ref<'default' | 'custom'>('default')
const tagsText = ref('')
const languages = ref<string[]>(['cpp', 'python'])
const testcaseInputMode = ref<'form' | 'json'>('form')
const testcaseRows = ref<TestcaseRow[]>([{ id: crypto.randomUUID(), input: '', output: '' }])
const testcaseText = ref('[\n  {\n    "input": "1 2\\n",\n    "output": "3\\n"\n  }\n]')
const functionInputMode = ref<'form' | 'json'>('form')
const functionDetailsText = ref(
  '{\n  "function_name": "solve",\n  "return_type": "int",\n  "params": [{"name": "nums", "type": "vector<int>"}]\n}',
)
const functionName = ref('solve')
const functionReturnType = ref('int')
const functionArguments = ref<FunctionArgumentRow[]>([{ id: crypto.randomUUID(), name: 'nums', type: 'vector<int>' }])
const easyQuestionType = ref('single_choice')
const easyInputMode = ref<'form' | 'json'>('form')
const easyStandardAnswer = ref('A')
const easyOptionRows = ref<EasyOptionRow[]>([
  { id: crypto.randomUUID(), key: 'A', description: '' },
  { id: crypto.randomUUID(), key: 'B', description: '' },
  { id: crypto.randomUUID(), key: 'C', description: '' },
  { id: crypto.randomUUID(), key: 'D', description: '' },
])
const easyFullScore = ref('1')
const easyMinSelections = ref('1')
const easyMaxSelections = ref('1')
const easyAllowPartialCredit = ref(false)
const easyJsonText = ref(
  '{\n  "question_type": "single_choice",\n  "standard_answer": "A",\n  "metadata": {\n    "options": ["A", "B", "C", "D"],\n    "option_descriptions": {\n      "A": "选项 A",\n      "B": "选项 B",\n      "C": "选项 C",\n      "D": "选项 D"\n    },\n    "full_score": 1\n  }\n}',
)
const spjLanguage = ref('cpp')
const spjSourceCode = ref('// 返回 0 表示 Accepted，返回非 0 表示 Wrong Answer\nint main(int argc, char** argv) {\n  return 0;\n}\n')
const validatorIgnoreWhitespace = ref(true)
const validatorIgnoreCase = ref(false)
const validatorUnordered = ref(false)
const validatorTokenMode = ref(false)
const validatorFloat = ref(false)
const validatorFloatEpsilon = ref('0')
const resourceLimits = ref<Record<string, ResourceLimitForm>>({
  cpp: { timeValue: '1', timeUnit: 's', memoryValue: '50', memoryUnit: 'mb' },
  python: { timeValue: '2', timeUnit: 's', memoryValue: '100', memoryUnit: 'mb' },
})
const saving = ref(false)
const validating = ref(false)
const notices = ref<Array<{ id: string; type: 'success' | 'error'; message: string }>>([])

const languageOptions = [
  { label: 'C++', value: 'cpp' },
  { label: 'Rust', value: 'rust' },
  { label: 'Python 3', value: 'python' },
]
const sandboxOptions = computed(() => {
  if (judgeMode.value === 'easy' || judgeMethod.value === 'spj') {
    return [{ label: 'nsjail', value: 'nsjail', description: '通用默认沙盒，兼容所有语言与 SPJ。' }]
  }

  const nonWasmLanguage = languages.value.find((language) => !['cpp', 'rust'].includes(language))
  if (nonWasmLanguage) {
    return [{ label: 'nsjail', value: 'nsjail', description: '当前语言组合包含非 C++/Rust，只能使用 nsjail。' }]
  }

  return [
    { label: 'nsjail', value: 'nsjail', description: '原生编译 + nsjail，兼容性最好。' },
    { label: 'wasm', value: 'wasm', description: 'Wasm 轻量执行，仅限 C++/Rust 的 validator 路线。' },
    { label: 'nsjail_wasm', value: 'nsjail_wasm', description: 'Wasm 执行器也进入 nsjail，隔离更强。' },
  ] as const
})
const validatorFields = [
  {
    key: 'ignore_whitespace',
    label: '忽略空白字符',
    description: '像 cin >> 一样忽略空格、换行和制表符，适合大多数标准输出比对。',
  },
  {
    key: 'ignore_case',
    label: '忽略大小写',
    description: '把 Yes 和 yes 视为相同结果，只在题面明确允许时开启。',
  },
  {
    key: 'is_unordered',
    label: '忽略顺序',
    description: '将输出视为无序集合，排序后再比较，适合答案顺序不唯一的题目。',
  },
  {
    key: 'is_token_mode',
    label: '按 Token 比较',
    description: '按空格和换行切分成多个 Token 后比较，便于浮点或无序场景。',
  },
  {
    key: 'is_float',
    label: '浮点比较',
    description: '将输出按浮点数比较，并结合浮点误差范围允许数值误差。',
  },
] as const
const spjTemplates = {
  cpp: `#include <fstream>
#include <string>

int main(int argc, char** argv) {
  // argv[0] 是程序自身路径
  // argv[1] = input.txt
  // argv[2] = output.txt
  // argv[3] = answer.txt
  if (argc != 4) {
    return 1;
  }

  std::ifstream input(argv[1]);
  std::ifstream user_out(argv[2]);
  std::ifstream answer(argv[3]);
  (void)input;
  std::string user_line;
  std::string answer_line;

  std::getline(user_out, user_line);
  std::getline(answer, answer_line);

  return user_line == answer_line ? 0 : 1;
}
`,
  python: `import sys


def main() -> int:
    # sys.argv[0] 是脚本路径
    # sys.argv[1] = input.txt
    # sys.argv[2] = output.txt
    # sys.argv[3] = answer.txt
    if len(sys.argv) != 4:
        return 1

    with open(sys.argv[1], "r", encoding="utf-8") as _input:
        _ = _input.read(0)

    with open(sys.argv[2], "r", encoding="utf-8") as user_out:
        user_line = user_out.readline().strip()

    with open(sys.argv[3], "r", encoding="utf-8") as answer:
        answer_line = answer.readline().strip()

    return 0 if user_line == answer_line else 1


if __name__ == "__main__":
    raise SystemExit(main())
`,
} satisfies Record<'cpp' | 'python', string>
const spjPresetTemplates = {
  strict_compare: {
    cpp: spjTemplates.cpp,
    python: spjTemplates.python,
  },
  testlib_style: {
    cpp: `#include <fstream>
#include <string>
#include <vector>

// Testlib 风格：读取 input / user output / answer 三个文件，自行决定返回码
int main(int argc, char** argv) {
  if (argc != 4) {
    return 1;
  }

  std::ifstream inf(argv[1]);
  std::ifstream ouf(argv[2]);
  std::ifstream ans(argv[3]);

  std::string input_line;
  std::string user_line;
  std::string answer_line;
  std::getline(inf, input_line);
  std::getline(ouf, user_line);
  std::getline(ans, answer_line);

  if (user_line == answer_line) return 0;
  return 1;
}
`,
    python: `import sys


def main() -> int:
    if len(sys.argv) != 4:
        return 1

    with open(sys.argv[1], "r", encoding="utf-8") as inf:
        input_line = inf.readline().strip()

    with open(sys.argv[2], "r", encoding="utf-8") as ouf:
        user_line = ouf.readline().strip()

    with open(sys.argv[3], "r", encoding="utf-8") as ans:
        answer_line = ans.readline().strip()

    if user_line == answer_line:
        return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
`,
  },
} satisfies Record<'strict_compare' | 'testlib_style', Record<'cpp' | 'python', string>>

const defaultValidatorProfile = () => ({
  ignore_whitespace: true,
  ignore_case: false,
  is_unordered: false,
  is_token_mode: false,
  is_float: false,
  float_epsilon: 0,
})

const parseJsonText = (value: string, fieldName: string) => {
  try {
    return JSON.parse(value)
  } catch {
    throw new Error(`${fieldName} 不是合法 JSON`)
  }
}

const pushNotice = (type: 'success' | 'error', message: string) => {
  const id = crypto.randomUUID()
  notices.value.push({ id, type, message })
  window.setTimeout(() => {
    notices.value = notices.value.filter((notice) => notice.id !== id)
  }, 3200)
}

const applySpjTemplate = (language: 'cpp' | 'python') => {
  spjSourceCode.value = spjTemplates[language]
}

const applySpjPreset = (preset: 'strict_compare' | 'testlib_style') => {
  spjSourceCode.value = spjPresetTemplates[preset][spjLanguage.value as 'cpp' | 'python']
}

const applyValidatorPreset = (preset: 'default' | 'float' | 'unordered') => {
  validatorMode.value = preset === 'default' ? 'default' : 'custom'
  if (preset === 'default') {
    validatorIgnoreWhitespace.value = true
    validatorIgnoreCase.value = false
    validatorUnordered.value = false
    validatorTokenMode.value = false
    validatorFloat.value = false
    validatorFloatEpsilon.value = '0'
    return
  }
  if (preset === 'float') {
    validatorIgnoreWhitespace.value = true
    validatorIgnoreCase.value = false
    validatorUnordered.value = false
    validatorTokenMode.value = true
    validatorFloat.value = true
    validatorFloatEpsilon.value = '1e-6'
    return
  }
  validatorIgnoreWhitespace.value = true
  validatorIgnoreCase.value = false
  validatorUnordered.value = true
  validatorTokenMode.value = true
  validatorFloat.value = false
  validatorFloatEpsilon.value = '0'
}

const createRow = (input = '', output = ''): TestcaseRow => ({
  id: crypto.randomUUID(),
  input,
  output,
})

const createArgumentRow = (name = '', type = ''): FunctionArgumentRow => ({
  id: crypto.randomUUID(),
  name,
  type,
})

const createEasyOptionRow = (key = '', description = ''): EasyOptionRow => ({
  id: crypto.randomUUID(),
  key,
  description,
})

const currentValidatorConfig = () => ({
  ignore_whitespace: validatorIgnoreWhitespace.value,
  ignore_case: validatorIgnoreCase.value,
  is_unordered: validatorUnordered.value,
  is_token_mode: validatorTokenMode.value,
  is_float: validatorFloat.value,
  float_epsilon: Number(validatorFloatEpsilon.value || '0'),
})

const syncValidatorModeFromConfig = () => {
  const current = currentValidatorConfig()
  const defaults = defaultValidatorProfile()
  validatorMode.value =
    current.ignore_whitespace === defaults.ignore_whitespace &&
    current.ignore_case === defaults.ignore_case &&
    current.is_unordered === defaults.is_unordered &&
    current.is_token_mode === defaults.is_token_mode &&
    current.is_float === defaults.is_float &&
    current.float_epsilon === defaults.float_epsilon
      ? 'default'
      : 'custom'
}

const defaultResourceLimit = (language: string): ResourceLimitForm =>
  language === 'python'
    ? { timeValue: '2', timeUnit: 's', memoryValue: '100', memoryUnit: 'mb' }
    : { timeValue: '1', timeUnit: 's', memoryValue: '50', memoryUnit: 'mb' }

const toResourceLimitForm = (timeLimitMs: number, memoryLimitKb: number): ResourceLimitForm => ({
  timeValue:
    timeLimitMs % 1000 === 0 ? String(Math.max(1, timeLimitMs / 1000)) : String(Math.max(1, timeLimitMs)),
  timeUnit: timeLimitMs % 1000 === 0 ? 's' : 'ms',
  memoryValue:
    memoryLimitKb % 1024 === 0 ? String(Math.max(1, memoryLimitKb / 1024)) : String(Math.max(1, memoryLimitKb)),
  memoryUnit: memoryLimitKb % 1024 === 0 ? 'mb' : 'kb',
})

const resourceLimitsPayload = () =>
  Object.fromEntries(
    languages.value.map((language) => {
      const form = resourceLimits.value[language] ?? defaultResourceLimit(language)
      const timeBase = Number(form.timeValue || '0')
      const memoryBase = Number(form.memoryValue || '0')
      if (!Number.isFinite(timeBase) || timeBase <= 0) {
        throw new Error(`${language} 时间限制必须是正数`)
      }
      if (!Number.isFinite(memoryBase) || memoryBase <= 0) {
        throw new Error(`${language} 空间限制必须是正数`)
      }
      return [
        language,
        {
          time_limit_ms: form.timeUnit === 's' ? Math.round(timeBase * 1000) : Math.round(timeBase),
          memory_limit_kb: form.memoryUnit === 'mb' ? Math.round(memoryBase * 1024) : Math.round(memoryBase),
        },
      ]
    }),
  )

const getResourceLimitForm = (language: string) => resourceLimits.value[language] ?? defaultResourceLimit(language)
const easyAnswerPlaceholder = computed(() => {
  if (easyQuestionType.value === 'multiple_choice') {
    return '如 AB 或 ["A","B"]'
  }
  if (easyQuestionType.value === 'true_false') {
    return '如 TRUE / FALSE'
  }
  return '如 A'
})

const normalizeEasyStandardAnswer = () => {
  const raw = easyStandardAnswer.value.trim()
  if (easyQuestionType.value === 'true_false') {
    return raw.toUpperCase() || 'TRUE'
  }
  if (easyQuestionType.value === 'single_choice') {
    return raw.toUpperCase() || 'A'
  }
  if (!raw) {
    throw new Error('多选题标准答案不能为空')
  }
  if (raw.startsWith('[')) {
    const parsed = parseJsonText(raw, '多选题标准答案')
    if (!Array.isArray(parsed)) {
      throw new Error('多选题标准答案 JSON 必须是数组')
    }
    return parsed
  }
  return raw.toUpperCase()
}

const easyMetadataPayload = () => {
  if (easyQuestionType.value === 'true_false') {
    return {
      full_score: Number(easyFullScore.value || '1'),
    }
  }

  const normalizedOptions = easyOptionRows.value
    .map((row) => ({
      key: row.key.trim().toUpperCase(),
      description: row.description.trim(),
    }))
    .filter((row) => row.key)

  if (normalizedOptions.length < 2) {
    throw new Error('单选题和多选题至少需要 2 个选项')
  }

  const optionKeys = normalizedOptions.map((row) => row.key)
  const uniqueKeys = new Set(optionKeys)
  if (uniqueKeys.size !== optionKeys.length) {
    throw new Error('选项标识不能重复')
  }

  const descriptions = Object.fromEntries(normalizedOptions.map((row) => [row.key, row.description]))

  const metadata: Record<string, unknown> = {
    options: optionKeys,
    option_descriptions: descriptions,
    full_score: Number(easyFullScore.value || '1'),
  }

  if (easyQuestionType.value === 'single_choice') {
    metadata.min_selections = 1
    metadata.max_selections = 1
  } else {
    metadata.min_selections = Number(easyMinSelections.value || '1')
    metadata.max_selections = Number(easyMaxSelections.value || '1')
    metadata.allow_partial_credit = easyAllowPartialCredit.value
  }

  return metadata
}

const syncTextcasesToJson = () => {
  testcaseText.value = JSON.stringify(
    testcaseRows.value.map((row) => ({
      input: row.input,
      output: row.output,
    })),
    null,
    2,
  )
}

const syncJsonToTestcases = () => {
  const parsed = parseJsonText(testcaseText.value, '测试用例')
  if (!Array.isArray(parsed)) {
    throw new Error('测试用例 JSON 必须是数组')
  }

  testcaseRows.value = parsed.length
    ? parsed.map((item) =>
        createRow(String(item?.input ?? item?.input_data ?? ''), String(item?.output ?? item?.expected_output ?? '')),
      )
    : [createRow()]
}

const syncFunctionFormToJson = () => {
  functionDetailsText.value = JSON.stringify(
    {
      function_name: functionName.value,
      return_type: functionReturnType.value,
      params: functionArguments.value
        .map((argument) => ({
          name: argument.name,
          type: argument.type,
        }))
        .filter((argument) => argument.name || argument.type),
    },
    null,
    2,
  )
}

const syncJsonToFunctionForm = () => {
  const parsed = parseJsonText(functionDetailsText.value, '函数签名')
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new Error('函数签名 JSON 必须是对象')
  }

  functionName.value = String((parsed as Record<string, unknown>).function_name ?? 'solve')
  functionReturnType.value = String((parsed as Record<string, unknown>).return_type ?? 'int')
  const rawArguments = Array.isArray((parsed as Record<string, unknown>).params)
    ? ((parsed as Record<string, unknown>).params as Array<Record<string, unknown>>)
    : Array.isArray((parsed as Record<string, unknown>).arguments)
      ? ((parsed as Record<string, unknown>).arguments as Array<Record<string, unknown>>)
    : []
  functionArguments.value = rawArguments.length
    ? rawArguments.map((argument) =>
        createArgumentRow(String(argument.name ?? ''), String(argument.type ?? '')),
      )
    : [createArgumentRow()]
}

const syncEasyFormToJson = () => {
  easyJsonText.value = JSON.stringify(
    {
      question_type: easyQuestionType.value,
      standard_answer: normalizeEasyStandardAnswer(),
      metadata: easyMetadataPayload(),
    },
    null,
    2,
  )
}

const syncJsonToEasyForm = () => {
  const parsed = parseJsonText(easyJsonText.value, '简单题配置')
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    throw new Error('简单题 JSON 必须是对象')
  }

  const obj = parsed as Record<string, unknown>
  easyQuestionType.value = String(obj.question_type ?? 'single_choice')
  easyStandardAnswer.value =
    typeof obj.standard_answer === 'string' ? obj.standard_answer : JSON.stringify(obj.standard_answer ?? 'A')
  const metadata = (obj.metadata ?? {}) as Record<string, unknown>
  const options = Array.isArray(metadata.options) ? metadata.options : ['A', 'B', 'C', 'D']
  const descriptions = (metadata.option_descriptions ?? {}) as Record<string, unknown>
  easyOptionRows.value = options.map((option) =>
    createEasyOptionRow(String(option), String(descriptions[String(option)] ?? '')),
  )
  easyFullScore.value = String(metadata.full_score ?? 1)
  easyMinSelections.value = String(metadata.min_selections ?? 1)
  easyMaxSelections.value = String(metadata.max_selections ?? options.length)
  easyAllowPartialCredit.value = Boolean(metadata.allow_partial_credit ?? false)
}

const switchTestcaseMode = (mode: 'form' | 'json') => {
  try {
    if (mode === testcaseInputMode.value) return
    if (mode === 'json') {
      syncTextcasesToJson()
    } else {
      syncJsonToTestcases()
    }
    testcaseInputMode.value = mode
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '测试用例转换失败')
  }
}

const switchFunctionMode = (mode: 'form' | 'json') => {
  try {
    if (mode === functionInputMode.value) return
    if (mode === 'json') {
      syncFunctionFormToJson()
    } else {
      syncJsonToFunctionForm()
    }
    functionInputMode.value = mode
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '函数签名转换失败')
  }
}

const switchEasyMode = (mode: 'form' | 'json') => {
  try {
    if (mode === easyInputMode.value) return
    if (mode === 'json') {
      syncEasyFormToJson()
    } else {
      syncJsonToEasyForm()
    }
    easyInputMode.value = mode
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '简单题配置转换失败')
  }
}

const addTestcaseRow = () => {
  testcaseRows.value.push(createRow())
  syncTextcasesToJson()
}

const addFunctionArgument = () => {
  functionArguments.value.push(createArgumentRow())
  syncFunctionFormToJson()
}

const removeFunctionArgument = (rowId: string) => {
  functionArguments.value = functionArguments.value.filter((row) => row.id !== rowId)
  if (functionArguments.value.length === 0) {
    functionArguments.value = [createArgumentRow()]
  }
  syncFunctionFormToJson()
}

const removeTestcaseRow = (rowId: string) => {
  testcaseRows.value = testcaseRows.value.filter((row) => row.id !== rowId)
  if (testcaseRows.value.length === 0) {
    testcaseRows.value = [createRow()]
  }
  syncTextcasesToJson()
}

const addEasyOption = () => {
  const nextCharCode = 65 + easyOptionRows.value.length
  const defaultKey = nextCharCode <= 90 ? String.fromCharCode(nextCharCode) : `OPT${easyOptionRows.value.length + 1}`
  easyOptionRows.value.push(createEasyOptionRow(defaultKey, ''))
}

const removeEasyOption = (rowId: string) => {
  easyOptionRows.value = easyOptionRows.value.filter((row) => row.id !== rowId)
  if (easyOptionRows.value.length === 0) {
    easyOptionRows.value = [createEasyOptionRow('A', ''), createEasyOptionRow('B', '')]
  }
}

const fillForm = (problem: ApiProblem) => {
  problemId.value = problem.problem_id
  title.value = problem.title
  description.value = problem.statement_md || problem.description
  judgeMode.value = problem.judge_mode
  judgeQueue.value = (problem.judge_queue as '' | 'fast' | 'normal' | 'heavy' | undefined) ?? ''
  judgeMethod.value = (problem.judge_method as 'validator' | 'spj' | undefined) ?? 'validator'
  sandboxKind.value = problem.sandbox_kind ?? 'nsjail'
  tagsText.value = (problem.tags ?? []).join(', ')
  languages.value = problem.languages ?? []
  resourceLimits.value = Object.fromEntries(
    (problem.languages ?? []).map((language) => {
      const raw = problem.resource_limits?.[language]
      return [
        language,
        raw ? toResourceLimitForm(raw.time_limit_ms, raw.memory_limit_kb) : defaultResourceLimit(language),
      ]
    }),
  )
  testcaseText.value = JSON.stringify(problem.testcases ?? [], null, 2)
  syncJsonToTestcases()

  if (problem.function_details_json) {
    functionDetailsText.value = JSON.stringify(problem.function_details_json, null, 2)
    syncJsonToFunctionForm()
  }

  if (problem.easy_judger) {
    easyQuestionType.value = String(problem.easy_judger.question_type ?? 'single_choice')
    easyStandardAnswer.value =
      typeof problem.easy_judger.standard_answer === 'string'
        ? problem.easy_judger.standard_answer
        : JSON.stringify(problem.easy_judger.standard_answer ?? 'A')
    const metadata = (problem.easy_judger.metadata ?? {}) as Record<string, unknown>
    const descriptions = (metadata.option_descriptions ?? {}) as Record<string, unknown>
    const options = Array.isArray(metadata.options) ? metadata.options : ['A', 'B', 'C', 'D']
    easyOptionRows.value = options.map((option) =>
      createEasyOptionRow(String(option), String(descriptions[String(option)] ?? '')),
    )
    easyFullScore.value = String(metadata.full_score ?? 1)
    easyMinSelections.value = String(metadata.min_selections ?? 1)
    easyMaxSelections.value = String(metadata.max_selections ?? (easyQuestionType.value === 'single_choice' ? 1 : easyOptionRows.value.length))
    easyAllowPartialCredit.value = Boolean(metadata.allow_partial_credit ?? false)
    syncEasyFormToJson()
  }

  spjLanguage.value = problem.spj_language || 'cpp'
  spjSourceCode.value =
    problem.spj_source_code || spjTemplates[(problem.spj_language as 'cpp' | 'python' | undefined) ?? 'cpp']
  const judgeConfig = (problem.judge_config ?? {}) as Record<string, unknown>
  validatorIgnoreWhitespace.value = Boolean(judgeConfig.ignore_whitespace ?? true)
  validatorIgnoreCase.value = Boolean(judgeConfig.ignore_case ?? false)
  validatorUnordered.value = Boolean(judgeConfig.is_unordered ?? false)
  validatorTokenMode.value = Boolean(judgeConfig.is_token_mode ?? false)
  validatorFloat.value = Boolean(judgeConfig.is_float ?? false)
  validatorFloatEpsilon.value = String(judgeConfig.float_epsilon ?? 0)
  syncValidatorModeFromConfig()
}

const normalizeTestcases = () => {
  if (testcaseInputMode.value === 'form') {
    syncTextcasesToJson()
  } else {
    syncJsonToTestcases()
    syncTextcasesToJson()
  }
  return parseJsonText(testcaseText.value, '测试用例')
}

const normalizeFunctionDetails = () => {
  if (functionInputMode.value === 'form') {
    syncFunctionFormToJson()
  } else {
    syncJsonToFunctionForm()
    syncFunctionFormToJson()
  }
  return parseJsonText(functionDetailsText.value, '函数签名')
}

const buildPayload = () => {
  const payload: Record<string, unknown> = {
    problem_id: problemId.value || undefined,
    title: title.value,
    description: description.value,
    statement_md: description.value,
    judge_mode: judgeMode.value,
    judge_queue: judgeQueue.value,
    judge_method: judgeMode.value === 'easy' ? 'validator' : judgeMethod.value,
    sandbox_kind: judgeMode.value === 'easy' ? 'nsjail' : sandboxKind.value,
    tags: tagsText.value
      .split(',')
      .map((tag) => tag.trim())
      .filter(Boolean),
  }

  if (judgeMode.value === 'acm') {
    payload.languages = languages.value
    payload.resource_limits = resourceLimitsPayload()
    payload.testcases = normalizeTestcases()
  }

  if (judgeMode.value === 'functional') {
    payload.languages = languages.value
    payload.resource_limits = resourceLimitsPayload()
    payload.testcases = normalizeTestcases()
    payload.function_details_json = normalizeFunctionDetails()
  }

  if (judgeMode.value === 'easy') {
    if (easyInputMode.value === 'json') {
      syncJsonToEasyForm()
    } else {
      syncEasyFormToJson()
    }
    payload.easy_judger = {
      question_type: easyQuestionType.value,
      standard_answer: normalizeEasyStandardAnswer(),
      metadata: easyMetadataPayload(),
    }
  }

  if (judgeMode.value !== 'easy' && judgeMethod.value === 'validator' && validatorMode.value === 'custom') {
    payload.judge_config = currentValidatorConfig()
  }

  if (judgeMethod.value === 'spj') {
    payload.spj_language = spjLanguage.value
    payload.spj_source_code = spjSourceCode.value
  }

  return payload
}

const validateProblem = async () => {
  try {
    validating.value = true
    await api.validateProblem(buildPayload())
    pushNotice('success', '结构校验通过')
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '校验失败')
  } finally {
    validating.value = false
  }
}

const saveProblem = async () => {
  try {
    saving.value = true
    const payload = buildPayload()
    const response = isNew.value
      ? await api.createProblem(payload)
      : await api.updateProblem(String(route.params.problemId), payload)

    pushNotice('success', '题目保存成功')
    fillForm(response.problem)
    if (isNew.value) {
      await router.replace(`/admin/problems/${response.problem.problem_id}/edit`)
    }
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '保存失败')
  } finally {
    saving.value = false
  }
}

onMounted(async () => {
  if (isNew.value) return
  try {
    const response = await api.getProblem(String(route.params.problemId))
    fillForm(response.problem)
  } catch (err) {
    pushNotice('error', err instanceof Error ? err.message : '加载题目失败')
  }
})

watch(spjLanguage, (nextLanguage) => {
  const current = spjSourceCode.value.trim()
  if (current === spjTemplates.cpp.trim() || current === spjTemplates.python.trim() || current === '') {
    applySpjTemplate(nextLanguage as 'cpp' | 'python')
  }
})

watch(
  languages,
  (nextLanguages) => {
    const nextLimits: Record<string, ResourceLimitForm> = {}
    nextLanguages.forEach((language) => {
      nextLimits[language] = resourceLimits.value[language] ?? defaultResourceLimit(language)
    })
    resourceLimits.value = nextLimits
  },
  { deep: true },
)

watch(easyQuestionType, (nextType) => {
  if (nextType === 'true_false') {
    easyStandardAnswer.value = ['TRUE', 'FALSE'].includes(easyStandardAnswer.value.toUpperCase()) ? easyStandardAnswer.value.toUpperCase() : 'TRUE'
    return
  }
  if (easyOptionRows.value.length < 2) {
    easyOptionRows.value = [createEasyOptionRow('A', ''), createEasyOptionRow('B', '')]
  }
  if (nextType === 'single_choice') {
    easyMinSelections.value = '1'
    easyMaxSelections.value = '1'
    if (!easyStandardAnswer.value) {
      easyStandardAnswer.value = easyOptionRows.value[0]?.key || 'A'
    }
  } else if (!easyStandardAnswer.value) {
    easyStandardAnswer.value = easyOptionRows.value.slice(0, 2).map((row) => row.key).join('')
  }
})

watch(judgeMode, (nextMode) => {
  if (nextMode === 'easy') {
    judgeMethod.value = 'validator'
  }
})

watch(judgeMethod, (nextMethod) => {
  if (nextMethod === 'validator') {
    syncValidatorModeFromConfig()
  }
  if (nextMethod === 'spj') {
    sandboxKind.value = 'nsjail'
  }
})

watch(
  [
    validatorIgnoreWhitespace,
    validatorIgnoreCase,
    validatorUnordered,
    validatorTokenMode,
    validatorFloat,
    validatorFloatEpsilon,
  ],
  () => {
    if (judgeMethod.value === 'validator') {
      syncValidatorModeFromConfig()
    }
  },
)

watch(validatorMode, (nextMode) => {
  if (nextMode === 'default' && judgeMethod.value === 'validator') {
    applyValidatorPreset('default')
  }
})

watch([judgeMode, languages, judgeMethod], () => {
  const values = sandboxOptions.value.map((option) => option.value)
  if (!values.includes(sandboxKind.value)) {
    sandboxKind.value = 'nsjail'
  }
}, { deep: true })
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
        <span class="page-kicker">Problem Editor</span>
        <h2 class="page-title">{{ isNew ? '新建题目' : '编辑题目' }}</h2>
        <p class="page-subtitle">录题管理已经接到当前 OJ 后端，支持测试用例 JSON 录入和表单化逐组录入。</p>
      </div>
    </div>

    <section class="section-grid">
      <div class="span-7 editor-card">
        <div class="editor-block">
          <span class="eyebrow">基础信息</span>
          <input v-model="title" placeholder="题目标题" />
        </div>
        <div class="two-column">
          <div class="editor-block">
            <span class="eyebrow">题目模式</span>
            <select v-model="judgeMode">
              <option>acm</option>
              <option>functional</option>
              <option>easy</option>
            </select>
          </div>
          <div class="editor-block">
            <span class="eyebrow">题目标识</span>
            <input v-model="problemId" placeholder="problem-id" />
          </div>
        </div>
        <div class="two-column">
          <div v-if="judgeMode !== 'easy'" class="editor-block">
            <span class="eyebrow">判题队列</span>
            <select v-model="judgeQueue">
              <option value="">自动路由</option>
              <option value="fast">fast</option>
              <option value="normal">normal</option>
              <option value="heavy">heavy</option>
            </select>
          </div>
          <div v-if="judgeMode !== 'easy'" class="editor-block">
            <span class="eyebrow">判题方式</span>
            <select v-model="judgeMethod">
              <option value="validator">validator</option>
              <option value="spj">SPJ</option>
            </select>
          </div>
        </div>
        <div v-if="judgeMode !== 'easy'" class="editor-block">
          <div class="inline-header">
            <span class="eyebrow">执行后端</span>
            <span class="muted">C++ / Rust 可选 Wasm 或 nsjail+Wasm，SPJ 与其他语言固定走 nsjail。</span>
          </div>
          <select v-model="sandboxKind">
            <option v-for="option in sandboxOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
          <p class="muted">{{ sandboxOptions.find((option) => option.value === sandboxKind)?.description }}</p>
        </div>
        <div v-if="judgeMode !== 'easy' && judgeMethod === 'validator'" class="editor-block">
          <div class="inline-header">
            <span class="eyebrow">Validator 策略</span>
            <span class="muted">默认策略等价于常规标准判题，自定义时再展开高级配置。</span>
          </div>
          <div class="mode-switcher compact-switcher">
            <button
              class="mode-card compact-card"
              :class="{ active: validatorMode === 'default' }"
              type="button"
              @click="validatorMode = 'default'"
            >
              <strong>默认策略</strong>
              <span>忽略空白，适合绝大多数 ACM 题</span>
            </button>
            <button
              class="mode-card compact-card"
              :class="{ active: validatorMode === 'custom' }"
              type="button"
              @click="validatorMode = 'custom'"
            >
              <strong>自定义策略</strong>
              <span>大小写、浮点、无序输出等高级比对</span>
            </button>
          </div>
        </div>
        <div class="editor-block">
          <span class="eyebrow">题面 Markdown</span>
          <textarea v-model="description" placeholder="输入题目描述、输入输出说明、样例"></textarea>
        </div>
        <div class="editor-block">
          <span class="eyebrow">标签</span>
          <input v-model="tagsText" placeholder="数组, 哈希表, 网络" />
        </div>
      </div>

      <div class="span-5 editor-card">
        <div class="mode-switcher">
          <button class="mode-card" :class="{ active: judgeMode === 'acm' }" type="button" @click="judgeMode = 'acm'">
            <strong>ACM</strong>
            <span>代码题，录测试用例和支持语言</span>
          </button>
          <button
            class="mode-card"
            :class="{ active: judgeMode === 'functional' }"
            type="button"
            @click="judgeMode = 'functional'"
          >
            <strong>Functional</strong>
            <span>函数签名 + 测试用例</span>
          </button>
          <button class="mode-card" :class="{ active: judgeMode === 'easy' }" type="button" @click="judgeMode = 'easy'">
            <strong>EasyJudge</strong>
            <span>判断 / 单选 / 多选</span>
          </button>
        </div>

        <div v-if="judgeMode !== 'easy'" class="editor-block">
          <span class="eyebrow">支持语言</span>
          <div class="language-checks">
            <label v-for="option in languageOptions" :key="option.value" class="language-check">
              <input v-model="languages" type="checkbox" :value="option.value" />
              <span>{{ option.label }}</span>
            </label>
          </div>
        </div>

        <div v-if="judgeMode !== 'easy'" class="editor-block">
          <div class="inline-header">
            <span class="eyebrow">资源限制</span>
            <span class="muted">C++ 默认 1s / 50MB，Python 默认翻倍</span>
          </div>
          <div class="resource-limit-list">
            <div v-for="language in languages" :key="language" class="resource-limit-row">
              <strong>{{ language === 'cpp' ? 'C++' : language === 'rust' ? 'Rust' : 'Python 3' }}</strong>
              <div class="resource-limit-field">
                <input v-model="getResourceLimitForm(language).timeValue" type="number" min="1" />
                <select v-model="getResourceLimitForm(language).timeUnit">
                  <option value="ms">ms</option>
                  <option value="s">s</option>
                </select>
              </div>
              <div class="resource-limit-field">
                <input v-model="getResourceLimitForm(language).memoryValue" type="number" min="1" />
                <select v-model="getResourceLimitForm(language).memoryUnit">
                  <option value="kb">KB</option>
                  <option value="mb">MB</option>
                </select>
              </div>
            </div>
          </div>
        </div>

        <div v-if="judgeMode === 'functional'" class="editor-block">
          <div class="inline-header">
            <span class="eyebrow">Functional 签名</span>
            <div class="mini-switch">
              <button
                class="ghost-button mini-switch-button"
                :class="{ active: functionInputMode === 'form' }"
                type="button"
                @click="switchFunctionMode('form')"
              >
                表单
              </button>
              <button
                class="ghost-button mini-switch-button"
                :class="{ active: functionInputMode === 'json' }"
                type="button"
                @click="switchFunctionMode('json')"
              >
                JSON
              </button>
            </div>
          </div>

          <div v-if="functionInputMode === 'form'" class="function-form-list">
            <div class="two-column">
              <div class="editor-block">
                <span class="eyebrow">函数名</span>
                <input v-model="functionName" placeholder="solve" @input="syncFunctionFormToJson" />
              </div>
              <div class="editor-block">
                <span class="eyebrow">返回类型</span>
                <input v-model="functionReturnType" placeholder="int / bool / vector<int>" @input="syncFunctionFormToJson" />
              </div>
            </div>

            <div class="function-argument-list">
              <div v-for="(argument, index) in functionArguments" :key="argument.id" class="function-argument-row">
                <div class="function-argument-title">参数 {{ index + 1 }}</div>
                <div class="two-column">
                  <div class="editor-block">
                    <span class="eyebrow">参数名</span>
                    <input v-model="argument.name" placeholder="nums" @input="syncFunctionFormToJson" />
                  </div>
                  <div class="editor-block">
                    <span class="eyebrow">参数类型</span>
                    <input v-model="argument.type" placeholder="vector<int>" @input="syncFunctionFormToJson" />
                  </div>
                </div>
                <button class="ghost-button" type="button" @click="removeFunctionArgument(argument.id)">删除参数</button>
              </div>
            </div>

            <button class="ghost-button add-row-button" type="button" @click="addFunctionArgument">新增参数</button>
          </div>

          <textarea v-else v-model="functionDetailsText"></textarea>
        </div>

        <template v-if="judgeMode === 'easy'">
          <div class="editor-block">
            <div class="inline-header">
              <span class="eyebrow">简单题配置</span>
              <div class="mini-switch">
                <button
                  class="ghost-button mini-switch-button"
                  :class="{ active: easyInputMode === 'form' }"
                  type="button"
                  @click="switchEasyMode('form')"
                >
                  表单
                </button>
                <button
                  class="ghost-button mini-switch-button"
                  :class="{ active: easyInputMode === 'json' }"
                  type="button"
                  @click="switchEasyMode('json')"
                >
                  JSON
                </button>
              </div>
            </div>

            <div v-if="easyInputMode === 'form'" class="function-form-list">
              <div class="editor-block">
                <span class="eyebrow">题型</span>
                <select v-model="easyQuestionType">
                  <option value="true_false">判断题</option>
                  <option value="single_choice">单选题</option>
                  <option value="multiple_choice">多选题</option>
                </select>
              </div>
              <div class="editor-block">
                <span class="eyebrow">标准答案</span>
                <input v-model="easyStandardAnswer" :placeholder="easyAnswerPlaceholder" />
              </div>

              <div class="editor-block">
                <div class="inline-header">
                  <span class="eyebrow">选项配置</span>
                  <span class="muted">单选题和多选题支持动态增减选项</span>
                </div>
                <div v-if="easyQuestionType !== 'true_false'" class="choice-description-list">
                  <div v-for="option in easyOptionRows" :key="option.id" class="choice-description-row">
                    <input v-model="option.key" class="choice-key-input" placeholder="A" />
                    <input v-model="option.description" :placeholder="`${option.key || 'A'}. 请输入选项说明`" />
                    <button class="ghost-button" type="button" @click="removeEasyOption(option.id)">删除</button>
                  </div>
                  <button class="ghost-button add-row-button" type="button" @click="addEasyOption">新增选项</button>
                </div>
                <p v-else class="muted">判断题不需要选项描述，作答页会直接展示“正确 / 错误”。</p>
              </div>

              <div class="two-column">
                <div class="editor-block">
                  <span class="eyebrow">满分</span>
                  <input v-model="easyFullScore" type="number" min="0" step="0.5" />
                </div>
                <template v-if="easyQuestionType === 'multiple_choice'">
                  <div class="editor-block">
                    <span class="eyebrow">部分分</span>
                    <label class="language-check">
                      <input v-model="easyAllowPartialCredit" type="checkbox" />
                      <span>允许部分得分</span>
                    </label>
                  </div>
                </template>
              </div>

              <div v-if="easyQuestionType === 'multiple_choice'" class="two-column">
                <div class="editor-block">
                  <span class="eyebrow">最少可选</span>
                  <input v-model="easyMinSelections" type="number" min="1" :max="Math.max(1, easyOptionRows.length)" />
                </div>
                <div class="editor-block">
                  <span class="eyebrow">最多可选</span>
                  <input v-model="easyMaxSelections" type="number" min="1" :max="Math.max(1, easyOptionRows.length)" />
                </div>
              </div>
            </div>
            <textarea v-else v-model="easyJsonText"></textarea>
          </div>
        </template>
      </div>

      <div v-if="judgeMode !== 'easy'" class="span-12 editor-card">
        <div class="page-header">
          <div>
            <span class="eyebrow">Testcases</span>
            <h3 class="section-title">{{ judgeMode === 'acm' ? 'ACM 测试用例录入' : 'Functional 测试用例录入' }}</h3>
          </div>
          <div class="toolbar">
            <button
              class="ghost-button"
              type="button"
              :class="{ active: testcaseInputMode === 'form' }"
              @click="switchTestcaseMode('form')"
            >
              表单录入
            </button>
            <button
              class="ghost-button"
              type="button"
              :class="{ active: testcaseInputMode === 'json' }"
              @click="switchTestcaseMode('json')"
            >
              JSON 录入
            </button>
            <button class="ghost-button" type="button" :disabled="validating" @click="validateProblem">
              {{ validating ? '校验中...' : '校验结构' }}
            </button>
            <button class="action-button" type="button" :disabled="saving" @click="saveProblem">
              {{ saving ? '保存中...' : '保存题目' }}
            </button>
          </div>
        </div>

        <div v-if="judgeMethod === 'validator' && validatorMode === 'custom'" class="two-column validator-grid">
          <div class="editor-block">
            <div class="inline-header">
              <span class="eyebrow">Validator 配置</span>
              <div class="inline-actions">
                <button class="ghost-button preset-button" type="button" @click="applyValidatorPreset('default')">默认模板</button>
                <button class="ghost-button preset-button" type="button" @click="applyValidatorPreset('float')">浮点误差题</button>
                <button class="ghost-button preset-button" type="button" @click="applyValidatorPreset('unordered')">无序集合题</button>
              </div>
            </div>
            <div class="validator-options">
              <label
                v-for="field in validatorFields"
                :key="field.key"
                class="language-check validator-option"
                :title="field.description"
              >
                <input
                  v-if="field.key === 'ignore_whitespace'"
                  v-model="validatorIgnoreWhitespace"
                  type="checkbox"
                />
                <input
                  v-else-if="field.key === 'ignore_case'"
                  v-model="validatorIgnoreCase"
                  type="checkbox"
                />
                <input
                  v-else-if="field.key === 'is_unordered'"
                  v-model="validatorUnordered"
                  type="checkbox"
                />
                <input
                  v-else-if="field.key === 'is_token_mode'"
                  v-model="validatorTokenMode"
                  type="checkbox"
                />
                <input
                  v-else
                  v-model="validatorFloat"
                  type="checkbox"
                />
                <span>{{ field.label }}</span>
              </label>
            </div>
          </div>
          <div class="editor-block">
            <span
              class="eyebrow"
              title="仅在开启浮点比较时生效，表示允许的最大误差，例如 1e-6。"
            >浮点误差范围</span>
            <input v-model="validatorFloatEpsilon" placeholder="例如 1e-6" />
          </div>
        </div>

        <div v-if="judgeMethod === 'spj'" class="editor-block">
          <span class="eyebrow">SPJ 配置</span>
          <div class="two-column">
            <div class="editor-block">
              <span class="eyebrow">SPJ 语言</span>
              <select v-model="spjLanguage">
                <option value="cpp">cpp</option>
                <option value="python">python</option>
              </select>
            </div>
            <div class="editor-block">
              <span class="eyebrow">说明</span>
              <div class="spj-help">
                <p class="muted">Judger 会把 SPJ 作为独立程序执行，并固定传入 3 个参数：</p>
                <ol class="spj-arg-list">
                  <li><code>input.txt</code>：测试输入文件</li>
                  <li><code>output.txt</code>：用户程序输出文件</li>
                  <li><code>answer.txt</code>：标准答案文件</li>
                </ol>
                <p class="muted">
                  注意：业务参数只有 3 个。C/C++ 中的 <code>argc == 4</code>、Python 中的
                  <code>len(sys.argv) == 4</code>，是因为还包含了程序自身路径。
                </p>
                <p class="muted">
                  返回码约定：<code>0</code> 表示 Accepted，<code>1</code> 表示 Wrong Answer，其他返回码会视为判题失败。
                </p>
                <div class="inline-actions">
                  <button class="ghost-button" type="button" @click="applySpjTemplate(spjLanguage as 'cpp' | 'python')">
                    载入当前语言模板
                  </button>
                  <button class="ghost-button preset-button" type="button" @click="applySpjPreset('strict_compare')">
                    严格比对模板
                  </button>
                  <button class="ghost-button preset-button" type="button" @click="applySpjPreset('testlib_style')">
                    Testlib 风格模板
                  </button>
                </div>
              </div>
            </div>
          </div>
          <div class="spj-editor-shell">
            <CodeEditor v-model="spjSourceCode" :language="(spjLanguage as 'cpp' | 'python')" :tab-size="4" />
          </div>
          <div class="editor-block">
            <span class="eyebrow">SPJ 使用案例</span>
            <div class="spj-help">
              <p class="muted">示例场景：题目要求输出一行整数。SPJ 程序读取用户输出和标准输出的第一行，完全一致返回 <code>0</code>，否则返回 <code>1</code>。</p>
              <p class="muted">如果你想做更复杂的判分，比如部分分、格式容错、多行结构校验，就继续在这个模板基础上扩展即可。</p>
            </div>
          </div>
        </div>

        <div v-if="testcaseInputMode === 'form'" class="testcase-form-list">
          <div v-for="(row, index) in testcaseRows" :key="row.id" class="testcase-row">
            <div class="testcase-row-header">
              <strong>测试组 {{ index + 1 }}</strong>
              <button class="ghost-button" type="button" @click="removeTestcaseRow(row.id)">删除</button>
            </div>
            <div class="two-column">
              <div class="editor-block">
                <span class="eyebrow">Input</span>
                <textarea class="testcase-textarea" v-model="row.input" @input="syncTextcasesToJson"></textarea>
              </div>
              <div class="editor-block">
                <span class="eyebrow">Output</span>
                <textarea class="testcase-textarea" v-model="row.output" @input="syncTextcasesToJson"></textarea>
              </div>
            </div>
          </div>

          <button class="ghost-button add-row-button" type="button" @click="addTestcaseRow">录入下一组</button>
        </div>

        <div v-else class="editor-block">
          <textarea v-model="testcaseText"></textarea>
        </div>
      </div>

      <div v-else class="span-12 editor-card">
        <div class="page-header">
          <div>
            <span class="eyebrow">Easy Configuration</span>
            <h3 class="section-title">EasyJudge 录题配置</h3>
          </div>
          <div class="toolbar">
            <button class="ghost-button" type="button" :disabled="validating" @click="validateProblem">
              {{ validating ? '校验中...' : '校验结构' }}
            </button>
            <button class="action-button" type="button" :disabled="saving" @click="saveProblem">
              {{ saving ? '保存中...' : '保存题目' }}
            </button>
          </div>
        </div>
        <ul class="bullet-list">
          <li>Easy 模式不需要代码语言和常规测试用例，核心是题型、标准答案和元数据。</li>
          <li>后续如果你接 AI，可以在这里新增“一键生成选项 / 一键生成标准答案检查规则”。</li>
        </ul>
      </div>
    </section>
  </div>
</template>

<style scoped>
.mode-switcher {
  display: grid;
  gap: 10px;
}

.mode-card {
  display: grid;
  gap: 4px;
  padding: 14px 16px;
  text-align: left;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
  color: var(--text-soft);
}

.mode-card.active {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--text);
}

.compact-switcher {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.compact-card {
  min-height: auto;
  padding: 12px 14px;
}

.language-checks {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
}

.validator-options {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.resource-limit-list,
.choice-description-list {
  display: grid;
  gap: 12px;
}

.resource-limit-row {
  display: grid;
  grid-template-columns: 100px minmax(0, 1fr) minmax(0, 1fr);
  gap: 10px;
  align-items: center;
}

.resource-limit-field {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 88px;
  gap: 8px;
}

.choice-description-row {
  display: grid;
  grid-template-columns: 90px minmax(0, 1fr) auto;
  gap: 10px;
  align-items: center;
}

.choice-label {
  font-weight: 700;
  color: var(--accent-strong);
}

.choice-key-input {
  text-transform: uppercase;
}

.validator-option {
  cursor: help;
}

.language-check {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--panel-strong);
  font-size: 0.82rem;
  line-height: 1.1;
}

.language-check input[type='checkbox'] {
  width: 12px;
  height: 12px;
  margin: 0;
  flex: 0 0 12px;
}

.inline-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 10px;
}

.mini-switch {
  display: inline-flex;
  gap: 8px;
}

.mini-switch-button.active {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent-strong);
}

.function-form-list,
.function-argument-list {
  display: grid;
  gap: 14px;
}

.function-argument-row {
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
}

.function-argument-title {
  margin-bottom: 10px;
  font-weight: 700;
}

.testcase-form-list {
  display: grid;
  gap: 16px;
}

.testcase-row {
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--panel-strong);
}

.testcase-row-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}

.testcase-textarea {
  min-height: 108px;
  max-height: 108px;
  overflow: auto;
}

.add-row-button {
  justify-self: flex-start;
}

.toolbar .ghost-button.active {
  border-color: var(--accent);
  background: var(--accent-soft);
  color: var(--accent-strong);
}

.preset-button {
  padding: 8px 12px;
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

.spj-help {
  display: grid;
  gap: 8px;
}

.spj-arg-list {
  margin: 0;
  padding-left: 18px;
  color: var(--text-soft);
}

.spj-editor-shell {
  min-height: 320px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  overflow: hidden;
  background: var(--panel-strong);
}

.spj-editor-shell :deep(.code-editor-shell) {
  height: 320px;
}
</style>
