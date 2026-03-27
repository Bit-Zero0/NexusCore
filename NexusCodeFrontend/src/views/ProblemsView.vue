<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api, type ApiProblem } from '../lib/api'

const problems = ref<ApiProblem[]>([])
const error = ref('')

onMounted(async () => {
  try {
    const response = await api.listProblems()
    problems.value = response.problems
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载题库失败'
  }
})
</script>

<template>
  <div class="page">
    <div class="page-header">
      <div>
        <span class="page-kicker">Problemset</span>
        <h2 class="page-title">题库</h2>
        <p class="page-subtitle">先用稳定的信息密度跑通浏览和提交链路，再逐步接入真实搜索与过滤。</p>
      </div>
      <div class="toolbar">
        <button class="ghost-button" type="button">标签筛选</button>
        <button class="ghost-button" type="button">难度排序</button>
      </div>
    </div>

    <section class="table-shell">
      <p v-if="error" class="muted">{{ error }}</p>
      <table>
        <thead>
          <tr>
            <th>状态</th>
            <th>题目</th>
            <th>模式</th>
            <th>难度</th>
            <th>通过率</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="problem in problems" :key="problem.problem_id">
            <td>
              <span class="status-pill processing">
                未提交
              </span>
            </td>
            <td>
              <strong>{{ problem.title }}</strong>
              <div class="tag-row">
                <span v-for="tag in problem.tags" :key="tag" class="tag">{{ tag }}</span>
              </div>
            </td>
            <td><span class="tag">{{ problem.judge_mode }}</span></td>
            <td>--</td>
            <td>--</td>
            <td>
              <div class="inline-actions">
                <RouterLink class="ghost-button" :to="`/problems/${problem.problem_id}`">查看</RouterLink>
                <RouterLink class="action-button" :to="`/problems/${problem.problem_id}/submit`">提交</RouterLink>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>
