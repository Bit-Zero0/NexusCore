<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api, type ApiClusterStats, type ApiProblem, type ApiSubmission } from '../lib/api'

const problems = ref<ApiProblem[]>([])
const submissions = ref<ApiSubmission[]>([])
const clusterStats = ref<ApiClusterStats | null>(null)
const loading = ref(true)
const error = ref('')

const focusProblems = computed(() => problems.value.slice(0, 4))
const clusterNodeCount = computed(() => clusterStats.value?.total_nodes ?? 0)

onMounted(async () => {
  try {
    loading.value = true
    error.value = ''
    const [problemResponse, submissionResponse, statsResponse] = await Promise.all([
      api.listProblems(),
      api.listSubmissions(),
      api.getClusterStats(),
    ])
    problems.value = problemResponse.problems
    submissions.value = submissionResponse.submissions
    clusterStats.value = statsResponse
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载首页数据失败'
  } finally {
    loading.value = false
  }
})
</script>

<template>
  <div class="page">
    <section class="hero-card">
      <div>
        <span class="page-kicker">NexusCode Frontend</span>
        <h2 class="hero-title">一套面向中文 OJ 与 runtime 平台的统一前端工作台</h2>
        <p class="hero-description">
          这套前端已经开始直接对接当前 Rust 后端。我们先把 OJ 主链跑通，再逐步把它扩成整个 NexusCode 项目的统一前端入口。
        </p>

        <div class="hero-actions">
          <RouterLink class="action-button" to="/problems">开始刷题</RouterLink>
          <RouterLink class="ghost-button" to="/admin/problems/new">录入新题目</RouterLink>
          <RouterLink class="ghost-button" to="/admin/cluster">查看集群</RouterLink>
        </div>
      </div>

      <div class="hero-side">
        <div class="stat-card">
          <span class="card-label">题库规模</span>
          <strong>{{ problems.length }}</strong>
          <p class="muted">当前直接读取 `/api/v1/oj/problems`。</p>
        </div>
        <div class="stat-card">
          <span class="card-label">提交记录</span>
          <strong>{{ submissions.length }}</strong>
          <p class="muted">提交详情页当前使用 HTTP 轮询状态推进。</p>
        </div>
        <div class="stat-card">
          <span class="card-label">Runtime 节点</span>
          <strong>{{ clusterNodeCount }}</strong>
          <p class="muted">节点数据来自 gateway 的 runtime 注册表聚合接口。</p>
        </div>
      </div>
    </section>

    <p v-if="error" class="muted">{{ error }}</p>
    <p v-else-if="loading" class="muted">正在加载首页数据...</p>

    <section class="stat-grid">
      <div class="metric-card">
        <span class="card-label">体验方向</span>
        <strong>中文化</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">视觉风格</span>
        <strong>明亮纸感 + 深夜代码感</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">布局策略</span>
        <strong>工作台优先</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">实时链路</span>
        <strong>HTTP 轮询 + MQ</strong>
      </div>
    </section>

    <section class="section-grid">
      <div class="span-7 table-shell">
        <div class="page-header">
          <div>
            <span class="eyebrow">重点题目</span>
            <h3 class="section-title">优先打通的真实后端对象</h3>
          </div>
          <RouterLink class="ghost-button" to="/problems">全部题目</RouterLink>
        </div>

        <table>
          <thead>
            <tr>
              <th>题目</th>
              <th>模式</th>
              <th>难度</th>
              <th>通过率</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="problem in focusProblems" :key="problem.problem_id">
              <td>
                <strong>{{ problem.title }}</strong>
                <span class="muted">{{ problem.problem_id }}</span>
              </td>
              <td><span class="tag">{{ problem.judge_mode }}</span></td>
              <td>--</td>
              <td>--</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="span-5 detail-card">
        <div>
          <span class="eyebrow">当前阶段</span>
          <h3 class="section-title">这一版前端先承接四件事</h3>
        </div>

        <ul class="bullet-list">
          <li>题库浏览与题目详情页，已经接到当前 `/api/v1/oj/problems`。</li>
          <li>代码提交页与结果页，当前走提交后自动调度加详情轮询。</li>
          <li>录题后台，继续适配当前 Problem / JudgeConfig 模型。</li>
          <li>集群监控页，当前已接到 gateway 的 runtime 节点聚合接口。</li>
        </ul>
      </div>
    </section>
  </div>
</template>
