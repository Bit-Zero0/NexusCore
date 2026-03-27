<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api, type ApiSubmission } from '../lib/api'

const submissions = ref<ApiSubmission[]>([])
const error = ref('')

onMounted(async () => {
  try {
    const response = await api.listSubmissions()
    submissions.value = response.submissions
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载提交记录失败'
  }
})
</script>

<template>
  <div class="page">
    <div class="page-header">
      <div>
        <span class="page-kicker">Submissions</span>
        <h2 class="page-title">提交记录</h2>
        <p class="page-subtitle">这里直接读取当前 OJ 提交记录，并继续向更完整的筛选与搜索能力演进。</p>
      </div>
    </div>

    <section class="table-shell">
      <p v-if="error" class="muted">{{ error }}</p>
      <table>
        <thead>
          <tr>
            <th>提交 ID</th>
            <th>题目</th>
            <th>语言</th>
            <th>状态</th>
            <th>队列</th>
            <th>时间 / 内存</th>
            <th>创建时间</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="submission in submissions" :key="submission.submission_id">
            <td>
              <RouterLink :to="`/submissions/${submission.submission_id}`">
                <strong>{{ submission.submission_id }}</strong>
              </RouterLink>
            </td>
            <td>{{ submission.problem_title }}</td>
            <td>{{ submission.language }}</td>
            <td>
              <span
                class="status-pill"
                :class="
                  submission.status === 'accepted'
                    ? 'accepted'
                    : submission.status === 'runtime_error'
                      ? 'runtime-error'
                      : 'processing'
                "
              >
                {{ submission.status }}
              </span>
            </td>
            <td><span class="tag">{{ submission.route_lane }}</span></td>
            <td>{{ submission.time_used_ms ?? '--' }} / {{ submission.memory_used_kb ?? '--' }}</td>
            <td>{{ new Date(submission.created_at_ms).toLocaleString('zh-CN') }}</td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>
