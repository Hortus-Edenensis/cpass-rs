# 验证记录

日期：2026-10-03。验证对象是当前 Rust 重构工作树及保留的 Python 严格解析基线。Rust 代码、离线回归、发布产物和平台验收分别记录；以下待验项不代表已经通过。

## 已确认结果

| 检查 | 环境 | 结果 | 证明范围 |
|---|---|---|---|
| Rust library tests | 本机 Rust 1.97.1 | 61 项通过 | 当前库模块单元测试；包括解析、匹配、签名、会话、任务、资源/批阅、日志、OCR 进程及通知回执的离线检查 |
| Rust CLI/HTTP integration | 本机 Rust 1.97.1 | 31 项通过 | 真实 localhost 请求、失败回执、动态考试参数、CLI、批量恢复、路径别名、离线零通知 |
| Python unittest | Python 3.10 | 35 项通过，0.303 秒 | 保留 Python 严格解析基线，离线 HTML/模拟搜索器/DTO |
| Python unittest | Python 3.11 | 35 项通过，0.258 秒 | 同一 Python 回归模块在第二个受支持版本运行 |

以上结果为本轮主执行器取得的运行结果。库测试通过不代替 CLI/HTTP 集成和在线平台验收。OCR 测试使用可控本地替代进程验证超时及输出边界，不能证明 Tesseract 对真实验证码的识别准确率。

## 最终检查清单

| 命令或检查 | 当前状态 | 通过标准 |
|---|---|---|
| `cargo test --locked --test rust_parity` | 31 项通过 | localhost 请求字段与回执检查通过，无真实平台/题库调用 |
| `cargo test --locked` | 通过：61 库 + 31 集成，文档测试 0 | 最终源码回归通过 |
| `cargo fmt --all --check` | 通过 | 最终源码无格式差异 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 最终源码无警告 |
| `cargo build --release --locked` | 待最终结果 | 最终源码 release 构建成功 |
| release CLI help/config/离线解析/恢复/TUI smoke | 待最终确认 | 示例命令参数有效，预期退出码和输出字段符合合同 |
| Python 改动文件格式检查 | 待最终确认 | 改动的 Python 文件符合项目格式工具 |
| Rust 1.88 最低版本 | CI 已加入 1.88.0，实际编译待验 | 锁定依赖及源码在 1.88 编译和回归通过；本机 1.97.1 通过不能代替 |
| Docker 构建与运行 | 待实跑证据 | 镜像构建成功；help、配置及离线流程可用，挂载可持久化 |
| Linux/macOS/Windows GitHub CI | 已配置，三平台运行待验 | 每个平台 fmt/clippy/test/release 全通过，产物可下载 |
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
