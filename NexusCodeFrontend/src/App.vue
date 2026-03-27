<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'

import { useThemeStore } from './stores/theme'

const route = useRoute()
const themeStore = useThemeStore()

const primaryNav = [
  { label: '题库', to: '/problems' },
  { label: '提交记录', to: '/submissions' },
  { label: '集群状态', to: '/admin/cluster' },
  { label: '录题管理', to: '/admin/problems' },
]

const currentSection = computed(() => {
  if (route.path.startsWith('/admin')) return '运营后台'
  if (route.path.startsWith('/submissions')) return '提交中心'
  return '在线判题'
})

const hideSidebar = computed(
  () =>
    route.path.includes('/submit') ||
    route.path.startsWith('/submissions') ||
    /^\/problems\/[^/]+$/.test(route.path) ||
    route.path.startsWith('/admin/problems'),
)

onMounted(() => {
  themeStore.initialize()
})
</script>

<template>
  <div class="app-shell">
    <div class="app-bg app-bg-top"></div>
    <div class="app-bg app-bg-bottom"></div>

    <header class="topbar">
      <RouterLink class="brand" to="/">
        <div class="brand-mark">N</div>
        <div>
          <div class="brand-title">Nexus OJ</div>
          <div class="brand-subtitle">中文分布式在线判题平台</div>
        </div>
      </RouterLink>

      <nav class="topnav">
        <RouterLink
          v-for="item in primaryNav"
          :key="item.to"
          class="topnav-link"
          :to="item.to"
        >
          {{ item.label }}
        </RouterLink>
      </nav>

      <button class="theme-toggle" type="button" @click="themeStore.toggleTheme()">
        <span>{{ themeStore.isDark ? '夜间模式' : '日间模式' }}</span>
        <strong>{{ themeStore.isDark ? 'Dark' : 'Light' }}</strong>
      </button>
    </header>

    <main class="page-shell" :class="{ 'page-shell-full': hideSidebar }">
      <aside v-if="!hideSidebar" class="sidebar">
        <div class="sidebar-card">
          <div class="eyebrow">当前区域</div>
          <h1>{{ currentSection }}</h1>
          <p>面向题目管理、代码提交、实时判题和 Judger 集群调度的统一前端入口。</p>
        </div>

        <div class="sidebar-card sidebar-grid">
          <div>
            <span class="metric-label">系统状态</span>
            <strong>在线</strong>
          </div>
          <div>
            <span class="metric-label">主题</span>
            <strong>{{ themeStore.isDark ? '夜间' : '日间' }}</strong>
          </div>
          <div>
            <span class="metric-label">支持题型</span>
            <strong>ACM / Easy / Functional</strong>
          </div>
          <div>
            <span class="metric-label">实时链路</span>
            <strong>HTTP 轮询 + MQ</strong>
          </div>
        </div>
      </aside>

      <section class="content-panel">
        <RouterView />
      </section>
    </main>
  </div>
</template>
