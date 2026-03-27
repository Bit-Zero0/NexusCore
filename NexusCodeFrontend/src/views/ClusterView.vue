<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'

import { api, type ApiClusterNode, type ApiClusterStats } from '../lib/api'

const stats = ref<ApiClusterStats | null>(null)
const error = ref('')
const loading = ref(true)
const autoRefreshEnabled = ref(true)
const refreshIntervalSec = 10
let refreshTimer: number | null = null

const loadStats = async () => {
  try {
    loading.value = true
    error.value = ''
    stats.value = await api.getClusterStats()
  } catch (err) {
    error.value = err instanceof Error ? err.message : '加载集群状态失败'
  } finally {
    loading.value = false
  }
}

const formatLane = (node: ApiClusterNode) => {
  const lanes = node.supported_lanes
  if (Array.isArray(lanes) && lanes.length > 0) {
    return lanes.join(' / ')
  }
  if (typeof lanes === 'string' && lanes.trim()) {
    return lanes
  }
  return '--'
}

const formatUtilization = (value: number) => `${Math.round((value || 0) * 100)}%`

const formatHeartbeat = (value?: number) => {
  if (!value) return '--'
  return new Date(value).toLocaleString('zh-CN')
}

const statusClass = (status: string) => {
  if (status === 'accepted' || status === 'online' || status === 'busy' || status === 'unhealthy') {
    return status
  }
  return 'processing'
}

const nodes = computed(() => stats.value?.nodes ?? [])

const startAutoRefresh = () => {
  if (refreshTimer !== null) {
    window.clearInterval(refreshTimer)
  }
  if (!autoRefreshEnabled.value) {
    refreshTimer = null
    return
  }
  refreshTimer = window.setInterval(() => {
    void loadStats()
  }, refreshIntervalSec * 1000)
}

const toggleAutoRefresh = () => {
  autoRefreshEnabled.value = !autoRefreshEnabled.value
  startAutoRefresh()
}

onMounted(async () => {
  await loadStats()
  startAutoRefresh()
})

onBeforeUnmount(() => {
  if (refreshTimer !== null) {
    window.clearInterval(refreshTimer)
    refreshTimer = null
  }
})
</script>

<template>
  <div class="page">
    <div class="page-header">
      <div>
        <span class="page-kicker">Cluster Stats</span>
        <h2 class="page-title">Runtime 集群监控</h2>
        <p class="page-subtitle">
          这里直接消费 gateway 的 runtime 节点注册表聚合数据，用来看 route 覆盖和节点健康度。
          当前每 {{ refreshIntervalSec }} 秒自动刷新一次。
        </p>
      </div>
      <div class="header-actions">
        <button class="ghost-button" type="button" @click="toggleAutoRefresh">
          {{ autoRefreshEnabled ? '暂停自动刷新' : '开启自动刷新' }}
        </button>
        <button class="ghost-button" type="button" :disabled="loading" @click="loadStats">
          {{ loading ? '刷新中...' : '刷新状态' }}
        </button>
      </div>
    </div>

    <p v-if="error" class="muted">{{ error }}</p>

    <section class="stat-grid">
      <div class="metric-card">
        <span class="card-label">在线节点</span>
        <strong>{{ stats?.online_nodes ?? '--' }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">路由活跃组</span>
        <strong>{{ stats?.busy_nodes ?? '--' }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">异常节点</span>
        <strong>{{ stats?.unhealthy_nodes ?? '--' }}</strong>
      </div>
      <div class="metric-card">
        <span class="card-label">平均负载</span>
        <strong>{{ stats ? formatUtilization(stats.avg_utilization) : '--' }}</strong>
      </div>
    </section>

    <section class="table-shell">
      <table>
        <thead>
          <tr>
            <th>节点 ID</th>
            <th>Route</th>
            <th>状态</th>
            <th>负载</th>
            <th>活跃任务</th>
            <th>容量</th>
            <th>TTL</th>
            <th>最近心跳</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="!loading && nodes.length === 0">
            <td colspan="8" class="empty-cell">当前没有可用节点快照</td>
          </tr>
          <tr v-for="node in nodes" :key="node.node_id">
            <td><strong>{{ node.node_id }}</strong></td>
            <td><span class="tag">{{ formatLane(node) }}</span></td>
            <td><span class="status-pill" :class="statusClass(node.status)">{{ node.status }}</span></td>
            <td>{{ formatUtilization(node.utilization) }}</td>
            <td>{{ node.active_tasks }}</td>
            <td>{{ node.capacity }}</td>
            <td>{{ node.ttl_sec ?? '--' }}</td>
            <td>{{ formatHeartbeat(node.last_heartbeat_ms) }}</td>
          </tr>
        </tbody>
      </table>
    </section>
  </div>
</template>

<style scoped>
.header-actions {
  display: inline-flex;
  gap: 10px;
}

.empty-cell {
  text-align: center;
  color: var(--text-mute);
}
</style>
