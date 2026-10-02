# cpass-rs

原生 Rust 超星课程、作业与考试客户端。详细阶段、能力矩阵、验收标准见 [ROADMAP.md](ROADMAP.md)。Python 基线保留在仓库中供对照和回退，原说明见 [docs/PYTHON-BASELINE.md](docs/PYTHON-BASELINE.md)。

答案采用严格策略：**完整且唯一才填，保留有效已有答案，只补未答部分**。四种题型统一匹配；未知题型、来源冲突、缺题和提交失败均阻止交卷。`false` 是有效判断答案，`null` 始终表示未答。Oaifree 已移除。

## 构建与验证

```bash
cargo build --release --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
./target/release/cpass --help
```

项目声明最低 Rust 1.88，使用 Rust 2024 edition；本机验证工具链为 Rust 1.97.1。最低版本与跨平台实际验证状态见 [VALIDATION.md](VALIDATION.md)。Rust 主业务运行不需要 Python、Poetry 或 OpenCV；可选 `ocr` 命令调用外部 Tesseract。

## 离线使用

```bash
cpass parse --kind work --input work.html --output questions.json --report parse-report.json
cpass parse --kind exam --input exam.html --output exam.json
cpass resolve --input questions.json --answers local-answers.json --output resolved.json --report resolve-report.json
```

`resolve` 只读取本地 JSON 题库，不调用配置中的在线搜索器，也不发送通知。题库为题干到答案的 JSON 对象，例如 `{"题干": "A", "判断题干": false}`。题干只规范化实体与排版空白，然后精确匹配。重复键或规范化后同题的不同有效答案会报告冲突。

题目 JSON 与旧版一致；考试导出 `type=0`，作业 `type=1`。`parse/resolve` 的题目 JSON 输出到 stdout，处理报告默认输出到 stderr；也可用 `--output/--report` 各自保存。未完成时返回非零，已解析结果仍会保存。上下标保留为 `^{…}` / `_{…}`，MathML 保留树结构，`math/tex` 内容保留在 `\(…\)` 边界内；不会将公式压成普通文字后匹配。

## 登录和多会话

配置默认读取当前目录 `config.yml`；`--config` 可选择其它文件，配置路径相对该文件解析。`--session` 选择会话存档，默认使用配置 session_path 下的 `default.json`。

```bash
cpass --config config.yml config-check
cpass sessions
export CPASS_PASSWORD='自己的密码'
cpass --session session/account1.json login --phone 13800000000
unset CPASS_PASSWORD
cpass --session session/account1.json login --qr
cpass --session session/account1.json account
cpass --session session/account1.json courses
```

密码仅从 `--password-env` 指定的环境变量读取，默认 `CPASS_PASSWORD`，不会写入存档。已有 Cookie 可以从 `CPASS_COOKIE` 用 `import-session` 导入；成功验证账户后才保存。默认打码姓名和手机号。存档兼容旧 Python Cookie 结构，保留域和有效期；Unix 文件权限为 `0600`。

短信和学号登录也使用同一会话存档：

```bash
cpass institutions --query '学校名称'
cpass --session session/account1.json sms-request --phone 13800000000
# 手工读取短信后设置 CPASS_SMS_CODE，再执行；不要把验证码写入命令参数。
cpass --session session/account1.json sms-login --phone 13800000000
unset CPASS_SMS_CODE
cpass --session session/student.json student-login --fid 123 --student-id 20260001
```

`student-login` 从 `CPASS_PASSWORD` 读取密码，`sms-login` 从 `CPASS_SMS_CODE` 读取验证码，可分别用 `--password-env/--code-env` 换名。短信请求默认国家码 `86`；有图形验证时先人工完成，再用 `--captcha-env` 指向验证结果。发送请求成功与登录成功分别报告。

## 课程、任务和作业

```bash
cpass chapters --course-id 123 --class-id 456
cpass tasks --course-id 123 --chapter-id 789 --output tasks.json
cpass run --course-id 123 --chapter-id 789 --output preview-report.json
cpass run --course-id 123 --chapter-id 789 --commit --output saved-report.json
cpass run --course-id 123 --chapter-id 789 --commit --final-submit --output final-report.json
```

同一课程多个班级时必须用 `--class-id` 指定。默认读取、匹配和导出；`--commit` 才汇报媒体任务或网络保存答案；`--final-submit` 才允许交作业。配置 enable 字段控制各任务执行。未知任务、坏任务卡和章节刷新失败会留在报告中，其它可获取任务继续。未执行或仅导出的任务仍计未完成，不能被章节总数掩盖；视频只有明确 `isPassed=true` 才计完成，`job=false` 不代表视频已完成。

从 `tasks.json` 选取一个 Work 对象保存为 `work-task.json`，也可独立运行：

```bash
cpass work --task-file work-task.json --output work-preview.json
cpass work --task-file work-task.json --commit --final-submit --output work-final.json
```

作业单题的 `cached` 表示本地缓存；`saved` 表示平台临时保存成功；`final_submitted` 表示最终交卷成功。这些字段与答案匹配数分别记录。失败时按旧 `fallback_save` 配置只保存可验证的字段；严格策略忽略 `fallback_fuzzer` 并明确输出提示。

批量课程按给定顺序运行，每门课程结束后写 checkpoint；重新运行会再次读取平台状态：

```bash
cpass run-batch --course-id 123 234 --resume checkpoint.json --output batch-preview.json
cpass run-batch --course-id 123 234 --resume checkpoint.json --commit --output batch-saved.json
cpass --session session/account1.json tui
```

checkpoint 校验当前 UID、课程列表和班级，不能跨账户或换课程复用；`--resume` 和 `--output` 必须是不同文件。恢复依据平台新读取的任务点状态跳过已完成课程，不将旧 checkpoint 的成功记录当作新回执。批量只处理课程任务，考试仍使用独立命令。TUI 是复用同一 CLI 业务的文本菜单，提供登录、账户、课程、章节、任务、课程执行和考试列表；写入和交作业分别确认。

直播、文章任务会返回官方学习链接。需要先在平台真人观看或阅读，再重跑命令刷新卡片；只有新读取的 `isPassed/job` 等状态确认完成后才计完成，不生成观看/阅读完成回执。

## 考试

```bash
cpass exams --course-id 123
cpass exam --course-id 123 --exam-id 321 --output exam-metadata.json
cpass exam --course-id 123 --exam-id 321 --start --output exam-preview.json
cpass exam --course-id 123 --exam-id 321 --start --commit --output exam-submissions.json
cpass exam --course-id 123 --exam-id 321 --start --commit --final-submit --output exam-final.json
```

`--start` 会请求开始考试并启动平台计时，预览也需要先开始。考试码使用 `--code-env CPASS_EXAM_CODE`。逐页核对题目 ID，保留已有答案，不重复单题提交；动态会话参数只在有效回执后更新。完整预览和答题卡边界、全部逐页确认、无失败且明确指定交卷开关时才最终交卷。

验证码与人脸要求返回 `action-required`，先完成真实验证。滑块/图形验证支持 `image-captcha` 与 `image-captcha-submit` 手动坐标接续；收到 validate 后通过 `--captcha-env` 使用。人脸用 `face-upload` 上传原始 JPEG，`course-face/exam-face` 获取实际平台回执；`exam-face --live-status` 必须来自真实客户端结果，将回执文件用 `--face-receipt` 传入考试。普通图片验证码用 `captcha-image/captcha-submit`。各子命令完整参数可用 `--help` 查询。

## 资源、批阅与人工审核导出

```bash
cpass resources --input work.html --base-url https://mooc1.chaoxing.com/work/view --output resources.json
cpass resources --input work.html --base-url https://mooc1.chaoxing.com/work/view --download images --output downloaded.json
cpass review-export --input reviewed.html --base-url https://mooc1.chaoxing.com/work/view --output reviewed.json
cpass work-export --task-file work-task.json --output work-resources.json
cpass work-export --task-file work-task.json --reviewed --output work-reviewed.json
cpass review --input questions.json --output review-drafts.json
cpass review --input questions.json --suggest --output suggested-drafts.json
```

图片 URL/alt、MathML、TeX 存放在独立 manifest，保留题号和选项位置，原题目 JSON 格式不变。`--base-url` 应为保存 HTML 的实际页面地址，用于解析相对资源 URL。只有 `--download` 才下载图片：独立无 Cookie 客户端、可信超星 HTTPS 主机、禁止跳转、每图最多 10 MiB，目标文件已存在则拒绝覆盖。

`work-export` 通过现有作业 GET 入口只读取得页面。批阅导出独立保留 `submitted_answer/reference_answer/score`，只有明确独立标签才提取，解释段不能用于猜答案；无法识别的字段保持空值或解析错误，部分结果先导出再报告未完成。已批阅真实 HTML 仍需按 [ROADMAP.md](ROADMAP.md) 的 M6 验收。

`review` 仅整理简答、名词解释、论述、计算、分录、资料题的人工审核材料；`--suggest` 才查询配置的搜索源生成草稿。输出明确标记 `requires_human_review=true` 和 `automatic_submission=false`，不会把草稿当成四题型有效答案，也不会自动提交主观题。

## OCR、日志与通知

```bash
cpass captcha-image --output captcha.png
cpass ocr --image captcha.png --output ocr-hint.json
# 人工核对图像与候选后设置 CPASS_CAPTCHA，单独提交。
cpass captcha-submit
cpass diagnose --output diagnostics.json
```

OCR 需要本机安装 Tesseract；默认可执行程序 `tesseract`、语言 `eng`、超时 10 秒。只接受不超过 5 MiB 的 PNG/JPEG，输出是字母数字提示且始终要求人工确认，不自动提交验证码。诊断命令默认读取 `log_path/events.jsonl`，可用 `--log` 指定；输出文件必须尚不存在。

```yaml
log_path: logs
log_retention_days: 30
ocr:
  executable: tesseract
  language: eng
  timeout_secs: 10
notifications:
  enabled: false
  gotify:
    url: https://gotify.example.com
    token_env: CPASS_GOTIFY_TOKEN
  mqtt:
    broker: mqtt://127.0.0.1:1883
    topic: cpass/events
    username_env: CPASS_MQTT_USER
    password_env: CPASS_MQTT_PASSWORD
    allow_plaintext: true
```

有有效配置文件时，业务命令记录受限字段的 JSONL 事件，不写 Cookie、密码、题干或原始平台响应。日志超过 3 MiB 后轮转；归档按 `log_retention_days` 清理（默认 30 天，范围 1–365）。诊断只接受已知事件字段，丢弃其它行并记录数量；Unix 日志与诊断文件权限为 `0600`。日志写入失败和通知失败不覆盖业务结果。通知成功及失败回执以独立 `notify` 阶段事件写入本地日志，可由 `diagnose` 查询；记录通知回执不会再次触发通知。

通知默认关闭，设置 `enabled: true` 后启用所配置的服务。Gotify 使用 HTTPS（本机测试可用 HTTP），token 从指定环境变量读入请求头，以有效消息 ID 确认接受。MQTT 当前实现 MQTT 3.1.1 的 `mqtt://` QoS 1，需要匹配的 PUBACK；没有原生 `mqtts://`，TLS 请通过本地代理接入。明文连接必须显式 `allow_plaintext: true`，带凭据只允许回环地址；凭据不写 broker URL。通知确认仅表示服务接受事件，不表示用户已看到。

`parse/resolve/resources/review-export/review/ocr` 只记录本地事件，不发送通知。资源下载和主观题搜索仍分别需要显式 `--download`、`--suggest`。

## 搜索器与兼容

保留配置名称：`jsonFileSearcher`、`sqliteSearcher`、`restApiSearcher`、`JsonApiSearcher`、`enncySearcher`、`cxSearcher`、`TiKuHaiSearcher`、`LyCk6Searcher`、`MukeSearcher`、`LemonSearcher`、`OpenAISearcher`、`OllamaSearcherAPI`。名称大小写兼容。

```yaml
searchers:
  - type: jsonFileSearcher
    file_path: local-answers.json
  - type: OpenAISearcher
    base_url: https://api.openai.com/v1/
    api_key: '$ENV:CPASS_API_KEY'
    model: your-model
```

现有 `config.yml` 的题库字段保持兼容；可添加每源 `timeout_secs`。使用独立 HTTP 客户端，不携带平台 Cookie；模型原文统一交由严格判定，不进行包含式猜选项或否定词改写。坏来源逐源隔离，`config-check` 在不联网的情况下发现初始化错误。

旧 TUI 参数保留可读取；Rust 的 `tui` 文本菜单复用 CLI。旧 `confirm_submit` 字段不取消显式交卷开关。原多会话目录、日志/导出/人脸路径字段保留；事件日志、题目导出和执行报告各自独立。API token、登录密码和通知凭据应使用环境变量，不写入配置值、命令参数或诊断材料。

## Docker 与发布

```bash
docker build -t cpass-rs .
docker run --rm cpass-rs --help
docker run --rm -v "$PWD:/data" cpass-rs --config /data/config.yml config-check
```

Docker 默认目录 `/data`，会话与配置通过挂载持久化。生产环境可用 `--user` 指定挂载目录的实际所有者。GitHub Actions 已配置 Linux、macOS、Windows 构建与产物上传；发布事件配置了 Rust 容器构建并推送本仓库 GHCR，实际执行状态以 [VALIDATION.md](VALIDATION.md) 和对应运行记录为准。Python 打包流程仅保留手动回退入口。

## 验证范围

已实现的协议来自现有 Python 业务链，回归使用离线 HTML 和 localhost HTTP。真实超星登录、风控、人脸、第三方服务现网、平台任务完成与批阅结果需要各自验收。答案能匹配、请求已保存、最终已交卷、平台判对分别报告，不以本地测试代替在线证据。

许可证沿用 GPL-3.0，见 [LICENSE](LICENSE)。
