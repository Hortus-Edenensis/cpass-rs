# cpass-rs

`cpass-rs` maintains the Python CxKitty client and a native Rust workspace.

The date-based Python release includes strict question parsing, conservative answer matching,
and DeepSeek V4.1 Flash thinking support. Download the Python source, Windows executable,
or native macOS application from [GitHub Releases](https://github.com/Hortus-Edenensis/cpass-rs/releases).

## Python runtime

Use Python 3.10 or 3.11 and Poetry 1.8:

```bash
python -m pip install poetry==1.8.5
poetry install --no-root
poetry run python main.py --self-check
poetry run python main.py
```

Edit `config.yml` before using the client. `--self-check` checks the installed parser, searcher,
and OCR dependencies offline and exits before login. Windows packages include `CxKitty.exe`;
macOS packages include `CxKitty.app`, which opens the interactive client in Terminal and stores
configuration/session data in `~/Library/Application Support/CxKitty`.

The four supported automatic question kinds require complete, unique answers. Existing valid
answers, including boolean `False`, are preserved. Missing/duplicate question IDs, parse errors,
partial answers, conflicts, or invalid save receipts block final submission.

### DeepSeek V4.1 Flash

Set `CPASS_OPENAI_API_KEY` in your environment and add this entry to `searchers` in `config.yml`:

```yaml
searchers:
  - type: OpenAISearcher
    base_url: "https://api.deepseek.com/v1"
    model: "deepseek-v4.1-flash"
    thinking: {type: enabled}
    reasoning_effort: high
    max_tokens: 8192
    response_format: {type: json_object}
    system_prompt: "只返回最终答案的 JSON 对象，不要解释。"
    prompt: "题型：{type}\n题目：{value}\n{options}"
```

The official endpoint maps `deepseek-v4.1-flash` to the current official API ID `deepseek-flash`;
gateway model names pass through unchanged. Only final `message.content` is matched;
`reasoning_content` is never used as an answer and truncated responses remain unresolved.
See [Python searcher configuration](./docs/python-searchers.md) and the
[official model update](https://api-docs.deepseek.com/updates/).

## Rust workspace status

- Native runtime: Rust CLI in [`crates/cpass-cli`](./crates/cpass-cli)
- Python runtime: implementation in [`cxapi/`](./cxapi), [`resolver/`](./resolver), and top-level `*.py`
- Current milestone: Phase 4 hardening and legacy-boundary follow-up
- Current runnable commands:
  - `cpass` now launches the top-level interactive orchestrator when invoked without a subcommand:
    it shows the banner, resolves or prompts for a session/login, prints account info, routes the
    user through course selection, and then drops into the same TUI-backed run flow for supported
    course execution
  - `cpass doctor`
  - `cpass config validate`
  - `cpass login`
  - `cpass courses list`
  - `cpass courses show`
  - `cpass tasks scan`
  - `cpass exam show`
  - `cpass exam export`
  - `cpass exam preview export`
  - `cpass run` drives the current Rust runtime path: it acknowledges reviewed fixture-backed
    legacy video/document/live task points, can expose chapter-work runtime snapshots plus
    answer-candidate preparation, and still blocks fail-closed before attendance, answer
    submission, or other unreviewed write-side flows
  - `cpass run --tui` wraps the same headless path in a CLI-owned event subscriber

## Repository Layout

- [`crates/cpass-core`](./crates/cpass-core): config, session, transport, parser, runners, and shared runtime contracts
- [`crates/cpass-cli`](./crates/cpass-cli): native Rust command-line runtime and output adapters
- [`fixtures/legacy`](./fixtures/legacy): legacy-derived fixtures for parser and compatibility tests
- [`pyproject.toml`](./pyproject.toml), [`poetry.lock`](./poetry.lock), and [`main.py`](./main.py): Python runtime, locked dependencies, and entrypoint
- [`docs/read-only-boundary.md`](./docs/read-only-boundary.md): allowlist and denylist for Phase 2 safe read-only endpoints
- [`docs/course-runner.md`](./docs/course-runner.md): current `CourseRunner` scope, plan contract, and unsupported execution behavior
- [`docs/task-executor-registry.md`](./docs/task-executor-registry.md): current executor registry scope, fail-closed queue-entry selection, and what still remains out of scope
- [`docs/exam-runner.md`](./docs/exam-runner.md): current `ExamRunner` scope, supported preview question kinds, and preview-based safety boundary
- [`docs/searcher-pipeline.md`](./docs/searcher-pipeline.md): shared Phase 3 answer-query pipeline plus the first local JSON and SQLite searcher backends
- [`docs/automation-profiles.md`](./docs/automation-profiles.md): profile-driven automation recipes plus the remaining interactive boundaries for login, run, and export flows
- [`docs/notification-pipeline.md`](./docs/notification-pipeline.md): CLI-owned notification summary and fan-out boundary for future Gotify/MQTT delivery
- [`docs/legacy-reference.md`](./docs/legacy-reference.md): Python maintenance and Rust protocol comparison boundaries

## Releases

Date tags such as `v2026.10.08` publish the Python source archive, Windows x86_64 executable,
and macOS arm64 / x86_64 application archives with SHA-256 checksums. Each frozen package
passes an offline startup check before publication.

## Build

The Rust CLI and its Docker image remain available for native workspace development.

### Local

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p cpass-cli -- --help
```

### Docker

```bash
docker build -t cpass-rs .
docker run --rm cpass-rs --help
```

## Configuration

The Rust CLI keeps compatibility with the legacy top-level path keys in [`config.yml`](./config.yml):

- `session_path`
- `log_path`
- `export_path`
- `face_image_path`
- `searchers`
- `notifications`

It also supports a Rust-side transport/login section, shown in [`config.example.yml`](./config.example.yml).

Named `profiles` can now layer non-interactive overrides on top of the root config for automation
use cases. Path, login, and transport fields inherit from the root config unless the selected
profile overrides them, while `searchers` and `notifications` replace the root lists only when the
profile sets them. Select a profile with `--profile <name>` or `CPASS_PROFILE=<name>`.
`config validate --json` and `doctor --json` now also surface `selected_profile` plus
`automation_paths` so scripts can inspect the effective automation directories without having to
reimplement profile selection or relative-path resolution.

Session-backed commands now resolve saved sessions in this order: `--phone`, the effective
`login.phone` after profile selection plus any `CPASS_PHONE` override, or the only saved session in
the effective `session_path`. They fail closed when multiple saved sessions exist without an
explicit selector, so automation profiles should pin a phone instead of relying on “latest
session” behavior.
[`docs/automation-profiles.md`](./docs/automation-profiles.md) now collects concrete profile recipes
for `config validate`, `doctor`, `login`, `exam export`, `exam preview export`, and headless
`run`, plus the remaining interactive boundaries that automation still has to respect.

`cpass run` can now instantiate the first supported local searcher backends from `searchers[*]`:

- `json` / legacy `jsonFileSearcher`: local JSON object where each key is a prompt or rendered
  search text and each value is one answer string or a string list
- `sqlite` / legacy `SqliteSearcher`: exact-match lookup against a local SQLite table, using
  `file_path` plus optional `table` / `req_field` / `rsp_field` overrides
- `http` / legacy `restApiSearcher` / `JsonApiSearcher`: outbound HTTP answer APIs with
  fail-closed config validation, request construction, and JSON answer-path extraction documented
  in [`docs/searcher-pipeline.md`](./docs/searcher-pipeline.md)
- `openai-compatible` / legacy `OpenAISearcher`: chat-completions-style answer APIs with
  validated `base_url` / `model` / `api_key`, DeepSeek V4.1 Flash thinking controls, prompt
  templating, optional fixture replay, and the same fail-closed chapter-work boundary

Candidate selection requires a complete, unambiguous answer for the four classic question
types and agreement among valid provider results. Invalid or conflicting answers remain unresolved.
Thinking traces are separate from final answers; truncated model responses cannot resolve a question.
See [`docs/searcher-pipeline.md`](./docs/searcher-pipeline.md) for DeepSeek configuration.

All four currently wired backends stay inside the same Phase 3 search-only boundary:
`cpass run` can prepare chapter-work answer candidates from them, but it still stops fail-closed
before any answer save or submit endpoint is called.

The Phase 4 notification path now reserves a validated `notifications[*]` config shape for
`gotify` and `mqtt` channels. The CLI actively delivers best-effort Gotify notifications for
`doctor`, `config validate`, `login`, and `run`. MQTT now supports the same summary payload through
either deterministic fixture replay or a one-shot live broker publish/disconnect flow for
`mqtt://` and `mqtts://` targets, while keeping the same best-effort warning boundary when broker
delivery fails. Lifecycle `RunEvent`s still fan into a CLI-owned summary collector first, which
keeps the delivery wiring additive and reversible instead of requiring another config migration or
pushing transport-specific concerns into `cpass-core`.

Sensitive values should come from environment variables:

- `CPASS_PHONE`
- `CPASS_PASSWORD`
- `CPASS_SESSION_DIR`
- `CPASS_LOG_DIR`
- `CPASS_EXPORT_DIR`
- `CPASS_FACE_DIR`
- `CPASS_REQUEST_TIMEOUT_SECS`
- `CPASS_REQUEST_RETRIES`
- `CPASS_OPENAI_API_KEY`

## Command Examples

Validate the normalized configuration:

```bash
cargo run -p cpass-cli -- config validate --json
```

Validate the same config through an automation-oriented profile override:

```bash
CPASS_PROFILE=automation cargo run -p cpass-cli -- config validate --json
```

Run an environment and filesystem check:

```bash
cargo run -p cpass-cli -- doctor --json
```

Create or refresh a session with password login:

```bash
export CPASS_PHONE="13800138000"
export CPASS_PASSWORD="your-password"
cargo run -p cpass-cli -- login --json
```

List courses using the selected saved session:

```bash
cargo run -p cpass-cli -- courses list --json
```

Show a single course snapshot using either `--course-id` or `--course-index`:

```bash
cargo run -p cpass-cli -- courses show --course-id 1001 --json
```

Scan chapter progress for a course:

```bash
cargo run -p cpass-cli -- tasks scan --course-id 1001 --json
```

`tasks scan` now includes a read-only task-point summary for each chapter when the chapter-card response is available.

When attachment metadata is available, `tasks scan` stays inside a narrow read-only boundary. It only reuses the read-only `GET /knowledge/cards` chapter-card snapshot to inspect embedded video/document resources, plus `GET /ananas/status/{objectId}` for video duration and stream metadata. It does not call video play-report endpoints such as `/multimedia/log/a`, document reading-report endpoints such as `/ananas/job/document`, or any other completion/progress reporting route that could mark a task point as finished or advance playback state.

That means the current attachment export is limited to metadata already exposed by those passive snapshots: resource identifiers, titles, job flags, document descriptors, and video status fields. If Chaoxing requires an active playback, reading, or reporting endpoint to reveal more detail, `cpass-rs` intentionally leaves that information out of Phase 2 `tasks scan`.

The exact Phase 2 allowlist and denylist live in [`docs/read-only-boundary.md`](./docs/read-only-boundary.md).

Build the current headless course run path, which can complete supported legacy video/document task points while keeping unsupported modules fail-closed:

```bash
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy_run_document run --course-id 1001 --json
```

`cpass run` still reuses the same read-only course resolution and task-scan flow as `courses list` plus `tasks scan`, then turns that snapshot into a deterministic `CourseRunPlan`. The JSON output includes both `execution_preflight`, a registry summary keyed by `execution_queue[*].queue_index`, and `execution_result`, the runtime snapshot produced by the headless queue driver after it starts dispatching the queue. Planning and runtime `events` are still buffered by the CLI-side output adapter instead of by `cpass-core`, and the CLI can now fan those same events out to a notification-summary collector without changing the runner boundary. That keeps the runner and driver independent from any concrete CLI, TUI, or future notification backend implementation while leaving event collection on the UI side of the boundary. The plan still includes both the nested chapter/task view and a flattened `execution_queue` that preserves the same stable ordering for later executors.

The current CLI path uses a mixed executor stack built from the core `HeadlessVideoCourseRunExecutor`, `HeadlessDocumentCourseRunExecutor`, the fail-closed live-task path, and the fail-closed chapter-work executor. It emits one legacy `/multimedia/log/a/{cpi}/{dtoken}` play-report acknowledgement for supported fixture-backed video entries, one legacy `/ananas/job/document` reading-report acknowledgement for supported fixture-backed document entries, blocks registered `live` entries before any runtime live endpoint is called, optionally prepares local JSON-backed chapter-work candidate selections, records those runtime events, and then still stops fail-closed before any chapter-work answer save or submit route is called. That means `cpass run` now exercises two real module-specific runtime paths plus two explicit fail-closed handoff paths while keeping later mutation blocked until the corresponding endpoint contracts are reviewed.

Subscribe to the same planning/runtime `RunEvent` stream through the thin CLI-owned TUI shell:

```bash
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy_run_document run --course-id 1001 --tui
```

If you prefer to start from the binary directly, invoking `cpass` with no subcommand now launches
the top-level interactive orchestrator. It shows the banner, resolves a saved session (or prompts
for password login or QR login when no session exists), lists available courses, accepts course
index / name / `course_id` selectors, and then drops into the same TUI-backed run path for
supported course execution:

```bash
cargo run -p cpass-cli --
```

The launcher also supports the legacy `EXAM|...` selector prefix in a read-only form. After the
course selector resolves, it shows the course exam list and accepts:

- exam index → render the same safe `exam show` snapshot inline
- `e<index>` → export the same safe `exam preview export` manifest
- `q` → exit the launcher

That route intentionally stays inside the current read-only safety boundary: it does not start,
submit, or otherwise mutate an exam session.

The default top-level interactive launcher is intentionally broader than `run --tui`: it owns the
banner, session/login prompts, and course routing before it hands a selected course off to the same
TUI-backed execution path. `cpass run --tui` still stays outside `cpass-core`. It does not inspect
transport state directly, build its own planner output, or add any new execution behavior. The
shell only subscribes to the existing `RunEvent` stream emitted by `CourseRunner`,
`CourseRunHeadlessDriver`, and the current mixed video/document/live/chapter-work executor path,
then renders planning status, queue-state transitions, warnings, and recent events on the CLI side.

When the interactive launcher hits a malformed JSON / HTML contract break from the backend or a
fixture, it now translates that failure back into the legacy relogin hint instead of dumping the
raw serde parse prefix directly into the terminal.

When stdout is a real terminal, the shell uses a lightweight alternate-screen refresh loop. When stdout is redirected or captured, it falls back to emitting one final plain-text snapshot so logs and offline integration tests stay deterministic. `--tui` cannot be combined with `--json`.

When `tasks scan` already exposed safe attachment metadata, `cpass run` now carries that same read-only snapshot forward into the plan so future executors can reuse video/document descriptors without adding a second scan pass or widening the safety boundary.

Known module kinds are currently limited to `insertvideo`, `insertdoc`, `insertlive`, and `work`. Any other task-point module is preserved in the output as `unknown(...)` so it can be fixture-captured and wired into a future executor without changing the planning contract. The planner now also preserves each chapter-card iframe's raw `data` JSON as `iframe_data` on both `task_points[*]` and `execution_queue[*]`, which keeps future module work such as live task-point support grounded in the original legacy snapshot instead of forcing a second chapter-card fetch. `execution_preflight.all_entries_registered` only tells you whether the registry recognized every planned queue entry; it does not mean the queue completed successfully. The current CLI can acknowledge fixture-backed `video` and `document` entries through the headless executor stack, while `live` and `chapter_work` still fall back to explicit fail-closed blocking before any unverified runtime endpoint is called. The full current contract and unsupported behaviors are documented in [`docs/course-runner.md`](./docs/course-runner.md).

Export the exam catalog for a course:

```bash
cargo run -p cpass-cli -- exam export --course-id 1001 --json
```

`exam export` includes catalog entries plus safe read-only cover metadata when the exam cover page is reachable.

Show a single exam snapshot using the same read-only catalog + cover metadata path:

```bash
cargo run -p cpass-cli -- exam show --course-id 1001 --exam-id 555001 --json
```

`exam show` stays within the same safe boundary as `exam export`: it resolves a course, reads the exam catalog, and fetches the existing read-only exam cover page for one exam. It does not call any route that would start an exam attempt, submit answers, or advance exam state.

Replay a read-only question inventory from a recorded exam preview snapshot:

```bash
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy exam preview export --course-id 1001 --exam-id 555001 --json
```

`exam preview export` is intentionally conservative. It only reads the exam list, the read-only cover page metadata, and the read-only `/exam-ans/exam/phone/preview` page. It does not call `/exam/phone/start`, single-question fetch endpoints, answer submission endpoints, or any other route that could start a timer, create a fresh attempt, or mutate state.

Because of that boundary, the command only works when the existing read-only metadata already contains the `exam_answer_id` needed to address a previously-entered preview snapshot. The fixture-backed path in `fixtures/legacy/` is the reference implementation for this Phase 2 export. If the cover page redirects to a completed or blocked flow, or if the platform does not expose enough preview parameters without entering the exam, the CLI fails closed instead of attempting a side-effecting request.

For the exact endpoint boundary that governs `courses`, `tasks`, and `exam` read-only commands, see [`docs/read-only-boundary.md`](./docs/read-only-boundary.md).

Replay read-only commands against recorded fixtures instead of the live network:

```bash
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy courses list --json
```

## CI and Release

The repository now uses Rust-focused automation:

- PR and `main` pushes run `cargo fmt --check`, `cargo clippy`, `cargo test`, `cargo build --release`, and a Docker smoke test
- tag builds produce release binaries for Linux, macOS, and Windows
- tag builds also push versioned Docker images

See:

- [`ci.yml`](./.github/workflows/ci.yml)
- [`release.yml`](./.github/workflows/release.yml)

## Legacy Boundary

The Python code is preserved because it still contains useful protocol knowledge and real-world behavior. It is not a supported runtime.

Use the legacy tree only for:

- capturing fixtures
- comparing outputs
- clarifying undocumented protocol behavior

Do not add net-new features to the Python path unless the work is strictly needed to support the Rust rewrite.

### Legacy Python workflow rules

Treat the Python tree as a lab bench, not as a second product surface:

- reach for Rust first whenever the outcome is a user-facing command, config contract, parser, transport, test, or documentation change
- use Poetry / the Python environment only when you need to capture fresh protocol fixtures, compare Rust behavior against a legacy response, or confirm how an undocumented legacy flow behaved
- land durable results in Rust-owned artifacts such as `fixtures/`, `crates/`, golden baselines, and docs; avoid leaving the final behavior encoded only in Python
- keep any Python-side changes narrowly scoped to capture helpers, fixture notes, or protocol annotations instead of shipping new automation features
- if a diff touches only Python files, it should also make the Rust-side handoff explicit (fixture capture, protocol comparison note, or follow-up contract); otherwise the change probably belongs under `crates/` or `docs/`

In practice, that means `cargo ...` remains the default setup and validation path for normal development, while
`poetry install` is now an opt-in legacy workflow reserved for fixture capture or protocol comparison sessions.
If a review ever sees a Python-only diff without a clear fixture-capture or protocol-comparison
handoff, treat that as a sign the change probably belongs under `crates/` or the Rust-owned docs
instead. [`docs/legacy-reference.md`](./docs/legacy-reference.md) is the canonical checklist for
that audit.

## Legal and Risk Notice

This repository is intended for protocol and automation research. It does not ship question-bank data. If you use it, you are responsible for your own environment, account, and compliance obligations.
