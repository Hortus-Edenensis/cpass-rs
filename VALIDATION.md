# 验证记录

日期：2026-10-03。验证对象是当前 Rust 重构工作树及保留的 Python 严格解析基线。Rust 代码、离线回归、发布产物和平台验收分别记录；以下待验项不代表已经通过。

## 已确认结果

| 检查 | 环境 | 结果 | 证明范围 |
|---|---|---|---|
| Rust library tests | 本机 Rust 1.97.1 | 61 项通过 | 当前库模块单元测试；包括解析、匹配、签名、会话、任务、资源/批阅、日志、OCR 进程及通知回执的离线检查 |
| Rust CLI/HTTP integration | 本机 Rust 1.97.1 | 31 项通过 | 真实 localhost 请求、失败回执、动态考试参数、CLI、批量恢复、路径别名、离线零通知 |
| Python unittest | Python 3.10 | 35 项通过，0.303 秒 | 保留 Python 严格解析基线，离线 HTML/模拟搜索器/DTO |
| Python unittest | Python 3.11 | 35 项通过，0.258 秒 | 同一 Python 回归模块在第二个受支持版本运行 |
| Python 格式 | Python 3.10/3.11；black、isort black profile、line length 100 | 两版本各 11 个改动文件通过 | 与项目 black 的 100 列约定一致 |
| 原生 release | macOS arm64，Rust 1.97.1 | 构建及 6 项 CLI 冒烟通过 | help、配置、作业/考试解析、未完成答案退出码、TUI 退出 |
| Docker | 本机 Linux arm64 与 GitHub Linux x64 | 本机 5 项无网络冒烟及容器 CI 通过 | help、配置、文件读写、本地匹配、真实 Tesseract 固定图片提示 |
| 日志/诊断 CLI | macOS arm64，源码 `8af026c` 的原生二进制 | 轮转、保留、过滤、权限与拒覆盖冒烟通过 | 临时目录测试，不代表用户目录长期运行或在线通知验收 |
| GitHub CI | Linux/macOS/Windows stable 与 Linux 1.88.0 | fmt/clippy/test/release 全部通过 | 锁定依赖、最低版本、Windows 路径别名回归及三平台 release CLI 冒烟 |

以上结果为本轮实际运行结果。已验证 Rust 源码提交 `8af026cdb427912a3d22835992af7bf0082f466c`；[四组 CI](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37046872221) 和 [容器 CI](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37046872191) 均成功。Unix 库测试 61 项，Windows 的本地 shell OCR 进程测试因平台条件跳过；CLI/HTTP 集成均为 31 项，Windows 构建执行了不存在目标文件的大小写路径别名条件回归。三平台 release CLI 冒烟在上述最新 CI 中通过。

OCR 单元测试使用可控替代进程验证超时；容器另以真实 Tesseract 5.3.0 识别固定 `CPASS 12345` 图片并要求人工确认。固定图片成功不能证明真实验证码识别准确率，也不表示平台接受验证。

日志 CLI 冒烟由交付目录的 `rust-refactor/log-smoke.py` 复现，结果见同目录 `log-smoke.json`：3.51 MiB 日志触发轮转；过期 `events-0.jsonl` 删除，未来数字归档和 `events-private.jsonl` 保留；新日志与诊断权限为 `0600`；270000 条无效行全部丢弃；既有诊断文件拒绝覆盖（退出码 1）且内容保留。

## 最终检查清单

| 命令或检查 | 当前状态 | 通过标准 |
|---|---|---|
| `cargo test --locked --test rust_parity` | 31 项通过 | localhost 请求字段与回执检查通过，无真实平台/题库调用 |
| `cargo test --locked` | 通过：61 库 + 31 集成，文档测试 0 | 最终源码回归通过 |
| `cargo fmt --all --check` | 通过 | 最终源码无格式差异 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 最终源码无警告 |
| `cargo build --release --locked` | 本机与四组 CI 通过 | release 构建成功 |
| release CLI help/config/离线解析/TUI smoke | 本机 6 项与最新三平台 release CLI 冒烟通过 | 预期退出码和输出字段符合合同；批量恢复由 31 项集成中的真实 CLI 检查覆盖 |
| Python 改动文件格式检查 | Python 3.10/3.11 的 black/isort 均通过，各 11 个文件 | isort 使用 black profile 与项目 100 列约定 |
| Rust 1.88 最低版本 | Linux 1.88.0 CI 全部通过 | 锁定依赖及源码在 1.88 编译和回归通过 |
| Docker 构建与运行 | 本机 5 项无网冒烟及 GitHub CI 通过 | 无网络容器 help、配置、离线文件读写和 OCR 成功 |
| Linux/macOS/Windows GitHub CI | 全部通过并上传产物 | 每个平台 fmt/clippy/test/release 全通过 |
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
