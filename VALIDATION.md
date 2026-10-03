# 验证记录

日期：2026-10-03。验证对象是当前 Rust 重构工作树及保留的 Python 严格解析基线。Rust 代码、离线回归、发布产物和平台验收分别记录；以下待验项不代表已经通过。

## 已确认结果

| 检查 | 环境 | 结果 | 证明范围 |
|---|---|---|---|
| Rust library tests | 本机 Rust 1.97.1 | 70 项通过 | 当前库模块单元测试；包括解析、匹配、签名、会话、任务、资源/批阅、日志、OCR 进程及通知回执的离线检查 |
| Rust CLI/HTTP integration | 本机 Rust 1.97.1 | 47 项通过 | 真实 localhost 请求、失败回执、动态考试参数、CLI、批量恢复、路径别名、离线零通知 |
| Python unittest | Python 3.10 | 35 项通过，0.303 秒 | 保留 Python 严格解析基线，离线 HTML/模拟搜索器/DTO |
| Python unittest | Python 3.11 | 35 项通过，0.258 秒 | 同一 Python 回归模块在第二个受支持版本运行 |
| Python 格式 | Python 3.10/3.11；black、isort black profile、line length 100 | 两版本各 11 个改动文件通过 | 与项目 black 的 100 列约定一致 |
| 原生 release | macOS arm64，Rust 1.97.1 | 本轮 release 构建通过，1 分 28 秒 | 本轮原生构建通过；发布包烟测回执随交付保存 |
| 公开资源下载 | 新原生二进制，无账户 | 官方公开 PNG 23472 字节；拒覆盖通过 | 公开 logo 传输及合成题号 42 关联，不代表真实题目 HTML 验收 |
| 日志/诊断 CLI | macOS arm64，源码 `122cb35` 的原生二进制（已有烟测） | 轮转、保留、过滤、权限与拒覆盖冒烟通过 | 临时目录测试，不代表用户目录长期运行或在线通知验收 |

本轮在 `122cb35` 基线上补充 mock 整体验收，发现并修复会话重启后设备标识变化：新会话存档保留完整移动 UA，旧 v1/ck 格式继续兼容；考试 IMEI 统一取自该会话。新增真实 localhost 及独立 CLI 进程验收覆盖四题型作业保存/交卷、四题型多页考试与动态 enc/设备参数、跨进程登录恢复、学号挑战保持旧会话、日志写入失败隔离。最终源码与 CI 来源由交付 manifest 分别记录。

本轮 `cargo fmt --all --check`、`cargo clippy --locked --all-targets -- -D warnings`、70 库 + 47 集成（共 117 项）已通过；库测试 6.78 秒、集成 5.60 秒。新增 1 项库测试和 5 项集成覆盖上述 mock 合同；密码登录恢复的设备标识回归在修复前失败、修复后通过。本轮 release 构建通过（1 分 28 秒）；最终 CI 单独记录。已有基线 CI（Linux/macOS/Windows stable、Linux Rust 1.88.0及容器）通过。
OCR 单元测试使用可控替代进程验证超时；此前已通过的容器另以真实 Tesseract 5.3.0 识别固定 `CPASS 12345` 图片并要求人工确认。固定图片成功不能证明真实验证码识别准确率，也不表示平台接受验证。

日志 CLI 冒烟由交付目录的 `rust-refactor/log-smoke.py` 复现，结果见同目录 `log-smoke.json`：3.51 MiB 日志触发轮转；过期 `events-0.jsonl` 删除，未来数字归档和 `events-private.jsonl` 保留；新日志与诊断权限为 `0600`；270000 条无效行全部丢弃；既有诊断文件拒绝覆盖（退出码 1）且内容保留。日志路径指向普通文件时，成功 parse 与未完成 resolve 的原退出码、题目 JSON 和报告均保留，原文件不覆盖；该分支已纳入 Rust CLI 回归。

公开资源烟测由交付目录的 `rust-refactor/resource-smoke.py` 复现，结果见 `resource-smoke.json`：下载 `https://passport2.chaoxing.com/images/fanya/readlogo.png` 得到 23472 字节，PNG 签名正确，SHA-256 为 `263ec62d5107642eae70a85a41374a4136f807d843d93860f37774a72072d5fc`；manifest 关联合成题号 42。重复目标拒绝覆盖（退出码 1），原文件保持。全程未使用账号，此证据只覆盖公开资源传输和合成关联。

## 最终检查清单

| 命令或检查 | 当前状态 | 通过标准 |
|---|---|---|
| `cargo test --locked --test rust_parity` | 47 项通过 | localhost 请求字段与回执检查通过，无真实平台/题库调用 |
| `cargo test --locked` | 通过：70 库 + 47 集成，文档测试 0 | 最终源码回归通过 |
| `cargo fmt --all --check` | 通过 | 最终源码无格式差异 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 最终源码无警告 |
| `cargo build --release --locked` | 本机通过，1 分 28 秒；最终 CI 回执随交付保存 | release 构建成功 |
| release CLI help/config/离线解析/TUI smoke | 原生六项烟测通过；TUI 三路径本机集成通过 | 预期退出码和输出字段符合合同；批量恢复由 47 项集成中的真实 CLI 检查覆盖 |
| Python 改动文件格式检查 | Python 3.10/3.11 的 black/isort 均通过，各 11 个文件 | isort 使用 black profile 与项目 100 列约定 |
| Rust 1.88 最低版本 | 业务源码 Linux 1.88.0 CI 通过 | 锁定依赖及源码在 1.88 编译和回归通过 |
| Docker 构建与运行 | 业务源码 GitHub 容器 CI 通过；本机最终烟测另附实际回执 | 无网络容器 help、配置、离线文件读写和 OCR 成功 |
| Linux/macOS/Windows GitHub CI | 业务源码四组 CI 通过；最终提交回执另附 | 每个平台 fmt/clippy/test/release 全通过 |
| M6 mock 业务验收 | 通过：四题型整卷、跨进程会话、挑战与日志故障回归 | 流程、实际请求字段、失败守卫通过；模拟回执不证明平台接受或批阅 |
| 真实平台 M6 | 待测试会话与明确执行范围 | 按 ROADMAP M6 每项保留真实回执与客户端对照 |

## 可复现命令

在仓库目录执行；Python 虚拟环境路径是本工作区的现有环境，独立克隆时应换成安装了项目依赖的对应 Python 解释器。

```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
../test-env310/bin/python -m unittest discover -s tests -p test_question_resolution.py -v
../test-env311/bin/python -m unittest discover -s tests -p test_question_resolution.py -v
./target/release/cpass --help
./target/release/cpass config-check
./target/release/cpass parse --kind work --input tests/fixtures/work_questions.html --output /tmp/cpass-work.json --report /tmp/cpass-parse-report.json
```

`config-check` 与离线答案解析不调用在线搜索器。`parse/resolve/resources/review-export/review/ocr` 不发送通知；`resources --download` 和 `review --suggest` 的主动联网由各自显式参数控制。离线回归即使配置启用了通知也验证零连接。

## 尚未宣称的结果

- 真实手机号/扫码/短信/学号登录、短信实际送达、图形验证解除、人脸验证与客户端活体结果。
- 真实课程视频/文档/直播/文章任务点完成，真实作业保存和交卷，授权测试考试逐题提交及最终交卷。
- 第三方题库可用性、自有模型现网服务质量、Gotify/MQTT 实际部署、Tesseract 实际识别率。
- 已批阅作业 HTML 的平台版本兼容性；当前批阅解析测试基于合成结构，遇到未知字段保留缺失与解析错误。
- 平台批阅正确率。答案完整可匹配、请求成功保存/提交、最终交卷成功、平台判对是四份不同证据。

每项新结果应补充实际命令、工具链、测试数量/退出状态和相关运行记录；不能仅凭存在代码、workflow 文件或示例命令把状态改为通过。

上一轮基线 CI： [四组 Rust 检查](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37052610296)、[容器检查](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37052610285)。最终提交及交付包来源以随附 manifest 和 CI 回执为准。
