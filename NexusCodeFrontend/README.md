# NexusOJFrontend

`NexusOJFrontend` 是 `Nexus_OJ` 的前端工作台，基于 `Vue 3 + Vite + Vue Router + TypeScript`。

## 当前页面

- `/problems`
  题库页
- `/problems/:problemId`
  题目详情
- `/problems/:problemId/submit`
  提交 / 作答页
- `/submissions`
  提交记录
- `/submissions/:submissionId`
  提交详情与实时结果
- `/admin/problems`
  录题管理
- `/admin/problems/new`
  新建题目
- `/admin/problems/:problemId/edit`
  编辑题目
- `/admin/cluster`
  集群监控

## 已实现能力

- LeetCode 风格提交页
- `CodeMirror 6` 代码编辑器
- 亮色 / 暗色主题
- 实时代码提交与 WebSocket 结果页
- 录题页支持：
  - ACM
  - Functional
  - EasyJudge
- Functional 录题支持表单 / JSON 双模式
- EasyJudge 录题支持表单 / JSON 双模式
- 集群状态真实接入 `/api/v1/cluster/stats`

## 环境变量

前端默认请求：

- `VITE_NEXUS_GATE_URL=http://127.0.0.1:8848`
- `VITE_NEXUS_GATE_TOKEN=replace-me`

可在 `.env.local` 中覆盖：

```bash
VITE_NEXUS_GATE_URL=http://127.0.0.1:8848
VITE_NEXUS_GATE_TOKEN=replace-me
```

## 开发

```bash
cd /home/fmy/Nexus_OJ/NexusOJFrontend
npm install
npm run dev
```

## 构建

```bash
cd /home/fmy/Nexus_OJ/NexusOJFrontend
npm run build
```

## 录题说明

### ACM / Functional

- 支持语言选择
- 支持按语言设置资源限制
- 支持测试用例录入
- 支持 Validator / SPJ

### EasyJudge

- 支持判断题 / 单选题 / 多选题
- 选项可动态增减
- 支持选项描述
- 支持 JSON 导入导出

## 注意事项

- 简单题不显示代码编辑器，直接在右侧作答
- 多选题标准答案支持：
  - `AB`
  - `["A","B"]`
- 提交详情页支持查看源码与复制代码
