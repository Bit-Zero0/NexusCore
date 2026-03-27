export type ProblemMode = 'acm' | 'easy' | 'functional'

export interface ProblemSummary {
  id: string
  title: string
  difficulty: '简单' | '中等' | '困难'
  mode: ProblemMode
  tags: string[]
  acceptance: string
  status?: '已通过' | '待提交' | '尝试过'
}

export interface SubmissionSummary {
  id: string
  problemId: string
  problemTitle: string
  language: string
  status: 'Accepted' | 'Judging' | 'Runtime Error' | 'Processing'
  lane: 'fast' | 'normal' | 'heavy'
  time: string
  memory: string
  createdAt: string
}

export interface ClusterNode {
  nodeId: string
  lane: 'fast' | 'normal' | 'heavy'
  status: 'online' | 'busy' | 'unhealthy'
  utilization: string
  activeTasks: number
  capacity: number
  heartbeat: string
}

export interface SupportedLanguage {
  value: 'cpp' | 'python'
  label: string
  runtime: string
}

export const problems: ProblemSummary[] = [
  {
    id: 'two-sum-stream',
    title: '双数之和流式判定',
    difficulty: '简单',
    mode: 'easy',
    tags: ['哈希表', 'EasyJudger', '前端示例'],
    acceptance: '89.3%',
    status: '已通过',
  },
  {
    id: 'nexus-top-k',
    title: '高并发日志中的 Top K',
    difficulty: '中等',
    mode: 'acm',
    tags: ['堆', '流处理', '分布式'],
    acceptance: '57.8%',
    status: '尝试过',
  },
  {
    id: 'json-parser-contract',
    title: 'JSON 解析器接口契约',
    difficulty: '中等',
    mode: 'functional',
    tags: ['函数签名', '单元校验', '模拟面试'],
    acceptance: '63.4%',
    status: '待提交',
  },
  {
    id: 'graph-latency-route',
    title: '最短延迟路由',
    difficulty: '困难',
    mode: 'acm',
    tags: ['图论', 'Dijkstra', '网络'],
    acceptance: '31.2%',
    status: '待提交',
  },
]

export const submissions: SubmissionSummary[] = [
  {
    id: 'sub-fast-0102',
    problemId: 'two-sum-stream',
    problemTitle: '双数之和流式判定',
    language: 'EasyJudger',
    status: 'Accepted',
    lane: 'fast',
    time: '2 ms',
    memory: '1.2 MB',
    createdAt: '2026-03-25 15:20',
  },
  {
    id: 'sub-normal-2201',
    problemId: 'nexus-top-k',
    problemTitle: '高并发日志中的 Top K',
    language: 'C++17',
    status: 'Judging',
    lane: 'normal',
    time: '--',
    memory: '--',
    createdAt: '2026-03-25 15:26',
  },
  {
    id: 'sub-heavy-7788',
    problemId: 'json-parser-contract',
    problemTitle: 'JSON 解析器接口契约',
    language: 'C++20',
    status: 'Processing',
    lane: 'heavy',
    time: '--',
    memory: '--',
    createdAt: '2026-03-25 15:29',
  },
]

export const clusterNodes: ClusterNode[] = [
  {
    nodeId: 'fast-node-1',
    lane: 'fast',
    status: 'online',
    utilization: '34%',
    activeTasks: 2,
    capacity: 8,
    heartbeat: '刚刚',
  },
  {
    nodeId: 'normal-node-1',
    lane: 'normal',
    status: 'busy',
    utilization: '81%',
    activeTasks: 6,
    capacity: 8,
    heartbeat: '3 秒前',
  },
  {
    nodeId: 'heavy-node-1',
    lane: 'heavy',
    status: 'online',
    utilization: '48%',
    activeTasks: 2,
    capacity: 4,
    heartbeat: '5 秒前',
  },
]

export const testcasePreview = [
  { name: 'case_01', status: 'Accepted', time: '1 ms', memory: '1.1 MB' },
  { name: 'case_02', status: 'Accepted', time: '2 ms', memory: '1.2 MB' },
  { name: 'case_03', status: 'Processing', time: '--', memory: '--' },
]

export const supportedLanguages: SupportedLanguage[] = [
  { value: 'cpp', label: 'C++', runtime: 'cpp / g++' },
  { value: 'python', label: 'Python 3', runtime: 'python / python3' },
]
