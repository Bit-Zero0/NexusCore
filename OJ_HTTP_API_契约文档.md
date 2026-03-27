# OJ HTTP API 契约文档

本文档描述当前 `nexus-oj` 已实现且已有自动化测试覆盖的 HTTP API 契约。

范围说明：
- 仅覆盖当前仓库中已经存在并稳定的接口。
- 仅描述当前真实返回结构，不把规划中的字段写成既成事实。
- 如代码与文档冲突，以当前代码和测试为准。

相关实现：
- 路由入口：[lib.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-oj/src/lib.rs)
- 领域模型：[domain.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-oj/src/domain.rs)
- 语言目录模型：[language.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-oj/src/language.rs)
- 运行时任务协议：[protocol.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-runtime/src/protocol.rs)

## 1. 基本约定

Base Path:

```text
/api/v1/oj
```

Content-Type:

```text
application/json
```

错误响应格式：

```json
{
  "code": "BAD_REQUEST",
  "message": "path problem_id does not match body problem_id"
}
```

当前错误码来源于共享错误模型，见 [error.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-shared/src/error.rs)：
- `BAD_REQUEST`
- `DATABASE_ERROR`
- `INVALID_CONFIG`
- `UNAUTHORIZED`
- `NOT_FOUND`
- `INTERNAL_ERROR`

## 2. 枚举与状态约定

### 2.1 JudgeMode

序列化为 `snake_case`：

- `acm`
- `functional`
- `easy_judge`

### 2.2 JudgeMethod

序列化为 `snake_case`：

- `validator`
- `spj`

说明：
- 默认判题也是 `validator`
- 旧的 `standard` 仅作为兼容别名接收，不再作为正式返回值

### 2.3 SubmissionStatus

序列化为 `snake_case`：

- `pending`
- `queued`
- `running`
- `accepted`
- `wrong_answer`
- `compile_error`
- `runtime_error`

当前语义说明：
- `POST /submissions` 当前返回的初始状态是 `pending`。
- `queued` / `running` / 最终判题状态来自后续 runtime 事件投影。

### 2.4 SubmissionCaseStatus

序列化为 `snake_case`：

- `accepted`
- `wrong_answer`
- `runtime_error`

## 3. 数据模型

### 3.1 ProblemSummary

```json
{
  "problem_id": "two-sum",
  "title": "Two Sum",
  "slug": "two-sum",
  "judge_mode": "acm"
}
```

### 3.2 ProblemDetail

```json
{
  "problem": {
    "problem_id": "two-sum",
    "title": "Two Sum",
    "slug": "two-sum",
    "judge_mode": "acm",
    "statement_md": "...",
    "supported_languages": ["cpp", "python", "rust"],
    "limits": {
      "cpp": {
        "time_limit_ms": 1000,
        "memory_limit_kb": 262144
      }
    },
    "testcases": [
      {
        "case_no": 1,
        "input": "1 2\n",
        "expected_output": "3\n",
        "is_sample": true,
        "score": 100
      }
    ],
    "judge_config": null,
    "easy_config": null
  }
}
```

### 3.3 SubmissionRecord

```json
{
  "submission_id": "sub_001",
  "problem_id": "two-sum",
  "user_id": "u1",
  "language": "cpp",
  "status": "pending",
  "score": 0,
  "max_score": 100,
  "message": null
}
```

### 3.4 SubmissionCaseResult

```json
{
  "case_no": 1,
  "status": "accepted",
  "score": 40,
  "time_used_ms": 8,
  "memory_used_kb": 0,
  "actual_output": "ok",
  "expected_output_snapshot": "",
  "message": null
}
```

### 3.5 SubmissionResult

```json
{
  "submission_id": "sub_001",
  "overall_status": "wrong_answer",
  "compile_output": null,
  "runtime_output": "runtime execution failed",
  "time_used_ms": 20,
  "memory_used_kb": 0,
  "judge_summary": "runtime execution failed",
  "case_results": []
}
```

### 3.6 SubmissionDetail

```json
{
  "submission": {
    "submission_id": "sub_001",
    "problem_id": "two-sum",
    "user_id": "u1",
    "language": "cpp",
    "status": "wrong_answer",
    "score": 40,
    "max_score": 100,
    "message": "runtime execution failed"
  },
  "source_code": "int main() { return 0; }",
  "result": {
    "submission_id": "sub_001",
    "overall_status": "wrong_answer",
    "compile_output": null,
    "runtime_output": "runtime execution failed",
    "time_used_ms": 20,
    "memory_used_kb": 0,
    "judge_summary": "runtime execution failed",
    "case_results": [
      {
        "case_no": 1,
        "status": "accepted",
        "score": 40,
        "time_used_ms": 8,
        "memory_used_kb": 0,
        "actual_output": "ok",
        "expected_output_snapshot": "",
        "message": null
      }
    ]
  }
}
```

说明：
- `result` 在刚创建提交、尚未收到 runtime 投影前可能为 `null`。
- `submission.status` 是聚合后的当前状态。
- `result.overall_status` 与 `submission.status` 在正常投影完成后应保持一致。

### 3.7 RuntimeTask

`GET /submissions/:submission_id/runtime-task` 返回 `nexus-runtime` 协议对象。

当前稳定字段：

```json
{
  "task_id": "task_sub_001",
  "task_type": "oj_judge",
  "source_domain": "oj",
  "source_entity_id": "sub_001",
  "queue": "oj_judge",
  "lane": "fast",
  "retry_policy": {
    "max_attempts": 3,
    "retry_delay_ms": 1000
  },
  "payload": {
    "kind": "oj_judge",
    "submission_id": "sub_001",
    "problem_id": "two-sum",
    "user_id": "u1",
    "language": "cpp",
    "judge_mode": "acm",
    "source_code": "int main() { return 0; }",
    "limits": {
      "time_limit_ms": 1000,
      "memory_limit_kb": 262144
    },
    "testcases": [],
    "judge_config": null
  }
}
```

当前语义说明：
- `source_domain` 固定为 `oj`
- `source_entity_id` 为 `submission_id`
- `queue` 当前固定为 `oj_judge`
- `lane` 由题目模式、语言、判题方式推导
- 默认重试策略为：
  - `max_attempts = 3`
  - `retry_delay_ms = 1000`

## 4. Catalog 接口

### 4.1 获取语言目录

```http
GET /api/v1/oj/catalog/languages
```

响应：

```json
[
  {
    "key": "cpp",
    "display_name": "C++",
    "source_extension": "cpp",
    "runtime_family": "native",
    "time_multiplier": 1,
    "memory_multiplier": 1,
    "sandbox_profile": "nsjail",
    "seccomp_policy": "cpp_default",
    "supported_modes": ["acm", "functional"]
  }
]
```

当前默认语言目录：
- `cpp`
- `python`
- `rust`

### 4.2 获取判题模式目录

```http
GET /api/v1/oj/catalog/judge-modes
```

响应：

```json
["acm", "functional", "easy_judge"]
```

### 4.3 获取代码模板

```http
GET /api/v1/oj/catalog/templates/:language/:mode
```

示例：

```http
GET /api/v1/oj/catalog/templates/cpp/acm
```

响应：

```json
{
  "language": "cpp",
  "judge_mode": "acm",
  "template": "#include <bits/stdc++.h>\nusing namespace std;\n..."
}
```

错误语义：
- 非法 `mode` 返回 `400 BAD_REQUEST`
- 不存在的 `language` 返回 `404 NOT_FOUND`

## 5. Problem 接口

### 5.1 获取题目列表

```http
GET /api/v1/oj/problems
```

响应：

```json
[
  {
    "problem_id": "two-sum",
    "title": "Two Sum",
    "slug": "two-sum",
    "judge_mode": "acm"
  }
]
```

### 5.2 获取题目详情

```http
GET /api/v1/oj/problems/:problem_id
```

响应：`ProblemDetail`

### 5.3 创建题目

```http
POST /api/v1/oj/problems
```

请求体：`Problem`

响应：`ProblemDetail`

### 5.4 更新题目

```http
PUT /api/v1/oj/problems/:problem_id
```

请求体：`Problem`

响应：`ProblemDetail`

约束：
- 路径参数 `problem_id` 必须与请求体中的 `problem.problem_id` 一致。
- 不一致时返回：

```json
{
  "code": "BAD_REQUEST",
  "message": "path problem_id does not match body problem_id"
}
```

## 6. Submission 接口

### 6.1 获取提交列表

```http
GET /api/v1/oj/submissions
```

响应：`SubmissionRecord[]`

### 6.2 创建提交

```http
POST /api/v1/oj/submissions
```

请求体：

```json
{
  "problem_id": "two-sum",
  "user_id": "u1",
  "language": "cpp",
  "source_code": "int main() { return 0; }"
}
```

响应：`SubmissionRecord`

当前稳定语义：
- 初始状态返回 `pending`
- 此接口负责创建提交记录
- 后续 runtime task 调度和状态推进由异步链路完成

### 6.3 获取提交详情

```http
GET /api/v1/oj/submissions/:submission_id
```

响应：`SubmissionDetail`

当前稳定语义：
- 若 runtime 尚未回写，`result` 可能为 `null`
- 若已完成投影，返回聚合后的 `SubmissionResult` 与 `SubmissionCaseResult[]`

### 6.4 获取提交对应 RuntimeTask

```http
GET /api/v1/oj/submissions/:submission_id/runtime-task
```

响应：`RuntimeTask`

用途：
- 用于确认 OJ 域向 runtime 发送的标准任务载荷
- 便于联调调度层、MQ 和 worker group 路由

## 7. Easy Judge 接口

### 7.1 创建 Easy Judge 提交

```http
POST /api/v1/oj/easy-judge/submissions
```

请求体：

```json
{
  "problem_id": "true-false-001",
  "user_id": "u1",
  "answer": "true"
}
```

或：

```json
{
  "problem_id": "multiple-choice-001",
  "user_id": "u1",
  "answer": ["A", "C"]
}
```

响应：`SubmissionRecord`

说明：
- `answer` 使用非标签联合类型
- 支持文本答案和选项数组两种序列化形式

## 8. 当前已被测试锁定的契约面

以下接口当前已有 HTTP 契约测试覆盖，见 [lib.rs](/home/fmy/Nexus_OJ/NexusCode/crates/nexus-oj/src/lib.rs)：

- `GET /api/v1/oj/catalog/languages`
- `GET /api/v1/oj/catalog/templates/:language/:mode`
- `GET /api/v1/oj/problems`
- `GET /api/v1/oj/problems/:problem_id`
- `POST /api/v1/oj/problems`
- `PUT /api/v1/oj/problems/:problem_id`
- `POST /api/v1/oj/submissions`
- `GET /api/v1/oj/submissions/:submission_id`
- `GET /api/v1/oj/submissions/:submission_id/runtime-task`
- `POST /api/v1/oj/easy-judge/submissions`

这意味着上面这些接口的核心请求路径、主要响应字段和关键语义，已经进入“应保持稳定”的阶段。

## 9. 当前未写入本契约的内容

以下内容暂不纳入当前稳定契约：
- 删除题目
- 题目发布/下线
- 标签、难度、分页、筛选
- 提交重判
- 面向前端的 WebSocket 事件契约
- runtime/gateway 运维 API

这些能力后续若实现，建议单独补充到平台 API 文档，不直接混入当前 OJ 主 REST 契约。
