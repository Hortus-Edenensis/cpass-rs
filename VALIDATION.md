# 验证记录

日期：2026-10-03。验证对象是当前 Rust 重构工作树及保留的 Python 严格解析基线。Rust 代码、离线回归、发布产物和平台验收分别记录；以下待验项不代表已经通过。

## 已确认结果

| 检查 | 环境 | 结果 | 证明范围 |
|---|---|---|---|
| Rust library tests | 本机 Rust 1.97.1 | 69 项通过 | 当前库模块单元测试；包括解析、匹配、签名、会话、任务、资源/批阅、日志、OCR 进程及通知回执的离线检查 |
| Rust CLI/HTTP integration | 本机 Rust 1.97.1 | 42 项通过 | 真实 localhost 请求、失败回执、动态考试参数、CLI、批量恢复、路径别名、离线零通知 |
| Python unittest | Python 3.10 | 35 项通过，0.303 秒 | 保留 Python 严格解析基线，离线 HTML/模拟搜索器/DTO |
| Python unittest | Python 3.11 | 35 项通过，0.258 秒 | 同一 Python 回归模块在第二个受支持版本运行 |
| Python 格式 | Python 3.10/3.11；black、isort black profile、line length 100 | 两版本各 11 个改动文件通过 | 与项目 black 的 100 列约定一致 |
| 原生 release | macOS arm64，Rust 1.97.1 | 新提交 release 构建通过，1 分 04 秒 | 当前 Rust 工作树本机构建；原生 CLI 六项烟测已通过 |
| 公开资源下载 | 新原生二进制，无账户 | 官方公开 PNG 23472 字节；拒覆盖通过 | 公开 logo 传输及合成题号 42 关联，不代表真实题目 HTML 验收 |
| 日志/诊断 CLI | macOS arm64，源码 `8fbfbd9` 的原生二进制 | 轮转、保留、过滤、权限与拒覆盖冒烟通过 | 临时目录测试，不代表用户目录长期运行或在线通知验收 |

以上本机结果对应业务源码（另含 8 项新增测试）提交 `8fbfbd9d42b23cc37a1860b6871b883cedbc8595`：`cargo fmt --all --check`、`cargo clippy --locked --all-targets -- -D warnings`、69 库 + 42 集成（共 111 项）及 release 构建均通过。新增回归覆盖公式保真、视频完成回执、未执行/仅导出任务状态、坏卡片和章节刷新失败接续、通知回执可诊断、TUI 三种真实 stdin 流程、已批阅只读导出和保存拒绝。业务源码 `8fbfbd9` 的 Linux/macOS/Windows stable 与 Linux Rust 1.88.0 CI、容器 CI 均已通过。追加 8 项回归在本机通过，最终提交的 CI 回执随交付目录保存。新增检查还验证真实 GET 超时/有限重试、开考不重放、旧会话与过期 Cookie、登录挑战、班级选择与数字章节排序、文档回执和 fresh 统计的区别、考试页异常及最终拒绝回执。

OCR 单元测试使用可控替代进程验证超时；此前已通过的容器另以真实 Tesseract 5.3.0 识别固定 `CPASS 12345` 图片并要求人工确认。固定图片成功不能证明真实验证码识别准确率，也不表示平台接受验证。

日志 CLI 冒烟由交付目录的 `rust-refactor/log-smoke.py` 复现，结果见同目录 `log-smoke.json`：3.51 MiB 日志触发轮转；过期 `events-0.jsonl` 删除，未来数字归档和 `events-private.jsonl` 保留；新日志与诊断权限为 `0600`；270000 条无效行全部丢弃；既有诊断文件拒绝覆盖（退出码 1）且内容保留。

公开资源烟测由交付目录的 `rust-refactor/resource-smoke.py` 复现，结果见 `resource-smoke.json`：下载 `https://passport2.chaoxing.com/images/fanya/readlogo.png` 得到 23472 字节，PNG 签名正确，SHA-256 为 `263ec62d5107642eae70a85a41374a4136f807d843d93860f37774a72072d5fc`；manifest 关联合成题号 42。重复目标拒绝覆盖（退出码 1），原文件保持。全程未使用账号，此证据只覆盖公开资源传输和合成关联。

## 最终检查清单

| 命令或检查 | 当前状态 | 通过标准 |
|---|---|---|
| `cargo test --locked --test rust_parity` | 42 项通过 | localhost 请求字段与回执检查通过，无真实平台/题库调用 |
| `cargo test --locked` | 通过：69 库 + 42 集成，文档测试 0 | 最终源码回归通过 |
| `cargo fmt --all --check` | 通过 | 最终源码无格式差异 |
| `cargo clippy --locked --all-targets -- -D warnings` | 通过 | 最终源码无警告 |
| `cargo build --release --locked` | 本机通过，1 分 04 秒；业务源码 CI 通过 | release 构建成功 |
| release CLI help/config/离线解析/TUI smoke | 原生六项烟测通过；TUI 三路径本机集成通过 | 预期退出码和输出字段符合合同；批量恢复由 42 项集成中的真实 CLI 检查覆盖 |
| Python 改动文件格式检查 | Python 3.10/3.11 的 black/isort 均通过，各 11 个文件 | isort 使用 black profile 与项目 100 列约定 |
| Rust 1.88 最低版本 | 业务源码 Linux 1.88.0 CI 通过 | 锁定依赖及源码在 1.88 编译和回归通过 |
| Docker 构建与运行 | 业务源码 GitHub 容器 CI 通过；本机最终烟测另附实际回执 | 无网络容器 help、配置、离线文件读写和 OCR 成功 |
| Linux/macOS/Windows GitHub CI | 业务源码四组 CI 通过；最终提交回执另附 | 每个平台 fmt/clippy/test/release 全通过 |
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

业务源码 CI： [四组 Rust 检查](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37051111590)、[容器检查](https://github.com/Hortus-Edenensis/cpass-rs/actions/runs/37051111594)。最终提交及交付包来源以随附 manifest 和 CI 回执为准。
