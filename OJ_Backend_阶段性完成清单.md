# OJ Backend 阶段性完成清单

## 1. 文档目的

本文档用于评估当前 `nexus-oj` 后端的完成度，并区分：

- 已完成
- 接近完成但仍需收口
- 可延期到下一阶段

这份清单以当前仓库代码为准，不以理想目标图为准。

---

## 2. 当前总体判断

当前 OJ 后端已经进入收尾阶段。

如果按“核心业务是否已经闭环”来判断，可以认为：

`OJ 后端核心能力已完成约 80% 到 90%`

原因是：

- 核心业务链已经成型
- 结果聚合和 runtime 边界已经稳定
- 主要剩余工作集中在接口稳定化、运维收口、平台级集群能力补强

---

## 3. 已完成

## 3.1 题目主链

已完成：

- 题目列表
- 题目详情
- 新建题目
- 更新题目
- 题目语言模板
- 判题模式枚举
- 题目配置校验

说明：

`Problem / ProblemDetail / ProblemSummary` 已经形成稳定模型，且应用层会对配置合法性做校验。

---

## 3.2 提交主链

已完成：

- 普通提交创建
- EasyJudge 提交创建
- 提交列表
- 提交详情
- 提交详情中的聚合结果返回
- 提交转 `RuntimeTask`

说明：

`SubmissionRecord / SubmissionDetail / SubmissionResult / SubmissionCaseResult` 已形成明确返回结构。

---

## 3.3 判题模式边界

已完成：

- ACM
- Functional
- EasyJudge
- Validator
- SPJ

说明：

业务规则已经留在 OJ 域，执行行为已迁移到 runtime 域，边界基本正确。

---

## 3.4 Runtime 集成链

已完成：

- OJ 生成 `RuntimeTask`
- 附带 `queue / lane / source_domain / retry_policy`
- runtime 执行事件回写 OJ
- OJ 投影 submission 状态
- OJ 聚合 case 级结果

说明：

这条链路已经是当前 OJ 后端最关键的主链之一，而且已经稳定可用。

---

## 3.5 接口契约测试

当前已补的 HTTP 契约测试包括：

- `POST /api/v1/oj/problems`
- `PUT /api/v1/oj/problems/:problem_id` 路径校验
- `POST /api/v1/oj/submissions`
- `GET /api/v1/oj/submissions/:submission_id`
- `GET /api/v1/oj/submissions/:submission_id/runtime-task`

这意味着核心 REST 面的主要返回结构已经开始被自动化测试锁定。

---

## 4. 接近完成但仍需收口

## 4.1 OJ 接口层仍缺少更完整的契约覆盖

虽然已经补了关键接口，但还可以继续补：

- `GET /api/v1/oj/problems`
- `GET /api/v1/oj/problems/:problem_id`
- `GET /api/v1/oj/catalog/languages`
- `GET /api/v1/oj/catalog/templates/:language/:mode`
- `POST /api/v1/oj/easy-judge/submissions`

这些不属于大功能缺失，但属于“收尾质量项”。

---

## 4.2 题目管理仍缺一部分 PRD 能力

PRD 中提到但当前未完成或未体现的能力包括：

- 删除题目
- 发布/下线题目
- 批量导入题目
- 题目标签、难度、题型管理

这些更偏管理后台能力，不影响当前 OJ 核心判题主链，但如果按 PRD 全量口径，还不算完成。

---

## 4.3 测试用例管理 UI/导入能力未形成

当前模型和存储已经支持 testcase 数据，但以下能力尚未体现：

- JSON 方式录入和表单录入互转
- testcase 结构化导入工具
- 数量限制与复杂校验的更完整策略

这部分更像“题库运营能力”，优先级可低于判题主链。

---

## 4.4 集群监控仍偏平台层，不完全属于 OJ 自身

OJ 已经能产出任务并消费结果，但集群监控这部分更多落在：

- runtime 节点
- gateway 聚合集群视图
- Redis 心跳
- RabbitMQ 队列状态

所以这部分虽然和 OJ 体验强相关，但不应再算作 OJ 域自身未完成，而应算平台执行层收尾。

---

## 5. 可延期到下一阶段

以下能力可以不阻塞 OJ 后端“阶段性收尾”，但后续一定要做：

1. 题目删除、发布、标签、难度等运营能力
2. 批量导题与题库迁移工具
3. 更完整的评测日志查看能力
4. 前端订阅与提交详情页面联调
5. Runtime 集群调度台
6. Function 域接入统一 runtime 的完整主链

---

## 6. 当前建议

如果目标是“先把 OJ 后端阶段性收尾”，建议按这个顺序继续：

1. 再补一轮 OJ 接口契约测试
2. 整理 OJ 接口返回结构文档
3. 确认前端真正依赖的字段集合
4. 然后把重心切到 runtime 集群调度与运维面

也就是说：

当前不应该再把主要时间花在 OJ 领域模型重写上，而应更多投入到：

- 契约稳定
- 平台执行层收尾
- 可观测性
- 集群运行能力

---

## 7. 当前结论

如果你问：

`OJ 模块后端是否快接近尾声了？`

答案是：

`是，核心业务已经很接近尾声。`

但如果问：

`整个 OJ 相关后端是否已经完全完成？`

答案是：

`还没有完全完成，但剩下的更多是收尾项和平台层能力，而不是 OJ 核心判题主链的大缺口。`

