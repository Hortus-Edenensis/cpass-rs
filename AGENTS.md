# AGENTS.md

## Purpose

This file is the execution checklist for the Rust-first reboot of `cpass-rs`.
It is intended for future agents and engineers working in this repository.

Rules for using this checklist:

- Treat the Python implementation as a legacy reference only.
- Prefer progressing the Rust workspace under `crates/`.
- Mark completed items by changing `- [ ]` to `- [x]`.
- Keep new work in the correct phase unless there is a strong reason to pull it forward.
- Do not mark an item complete unless code, tests, and docs are aligned.

## Current State

- Default implementation path: Rust
- Legacy implementation path: Python reference only
- Current delivery stage: Phase 3 in progress
- Current verified commands:
  - `cpass`
  - `cpass doctor`
  - `cpass config validate`
  - `cpass login`
  - `cpass courses list`
  - `cpass courses show`
  - `cpass tasks scan`
  - `cpass exam show`
  - `cpass exam export`
- `cpass exam preview export`
- `cpass run`
- `cpass run --tui`
- Current non-goal:
  - `cpass run` currently acknowledges legacy video/document/live task points and can expose fixture-backed chapter-work runtime snapshots, but live attendance/completion and chapter-work answer resolution/submission still stop fail-closed

## Validation Commands

Run these before closing any meaningful unit of work:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use these when validating the fixture-backed read-only paths:

```bash
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy --json courses list
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy --json tasks scan --course-id 1001
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy --json exam show --course-id 1001 --exam-id 555001
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy --json exam export --course-id 1001
cargo run -p cpass-cli -- --fixture-dir fixtures/legacy --json exam preview export --course-id 1001 --exam-id 555001
```

## Phase 0 — Delivery Bootstrap

- [x] Create a Rust workspace with `cpass-core` and `cpass-cli`.
- [x] Add a runnable binary entrypoint named `cpass`.
- [x] Replace Python-first CI with Rust quality gates.
- [x] Add release workflows for versioned binaries and Docker images.
- [x] Replace the Python runtime Docker image with a Rust multi-stage build.
- [x] Reposition the Python implementation as legacy reference in docs.
- [x] Add a root README that reflects the Rust-first reboot.

## Phase 1 — Core Contracts

- [x] Define `AppConfig` with config loading, validation, and environment overrides.
- [x] Keep compatibility with legacy top-level path fields from `config.yml`.
- [x] Define `SessionStore` and add file-based session persistence.
- [x] Add legacy session compatibility loading for old JSON session files.
- [x] Stop writing plaintext passwords in the new Rust session format.
- [x] Define `SecretSource` and wire environment-backed secret loading.
- [x] Define `ChaoxingTransport` and add a reqwest-backed implementation.
- [x] Define `RunEvent` and use it for CLI-side lifecycle reporting.
- [x] Implement `doctor`.
- [x] Implement `config validate`.
- [x] Implement `login` with password flow and session save.

## Phase 2 — Read-Only Parity

### Implemented

- [x] Implement `courses list` against the live course list endpoint.
- [x] Implement `tasks scan` with chapter progress summary.
- [x] Enrich `tasks scan` with read-only task-point summaries from chapter cards.
- [x] Implement `exam export` for exam catalog export.
- [x] Refresh account metadata from transport when available instead of relying only on legacy session placeholders.
- [x] Add fixture-backed transport for offline replay of read-only commands.
- [x] Add parser fixtures for course list, chapter list, chapter status, task cards, exam list, account info, and exam cover pages.
- [x] Add offline CLI integration tests for `courses list`, `tasks scan`, and `exam export`.
- [x] Add golden JSON baselines for the fixture-backed CLI outputs.
- [x] Enrich `exam export` with safe read-only exam cover metadata.

### Remaining

- [x] Add a read-only export for exam preview / question inventory without accidentally starting or mutating an exam session.
  - [x] Add parser models, a legacy preview fixture, and unit tests for question inventory captured from an already-entered exam session snapshot.
  - [x] Wire fixture-backed preview parsing through core transport APIs without calling start/submit exam endpoints.
  - [x] Add a fixture-backed CLI export plus golden and offline integration tests for preview question inventory.
  - [x] Document preview export limitations and the read-only safety assumptions behind the endpoint choice.
- [x] Add read-only task attachment metadata where safe, such as video duration or document resource details.
  - [x] Add parser models, legacy `knowledge/cards` fixtures, and unit tests for safe video/document attachment metadata extraction plus video status response parsing.
  - [x] Wire read-only attachment metadata fetching through core task-scan APIs without introducing write-side endpoints.
  - [x] Enrich fixture-backed `tasks scan` CLI output, golden baselines, and offline integration tests with attachment metadata.
  - [x] Document which attachment metadata routes are treated as safe read-only and which remain out of scope.
  - [-] Add parser models, legacy attachment fixtures, and unit tests for safe task attachment snapshots from `knowledge/cards` and video status responses. Replaced by the more specific completed parser subtask above.
  - [-] Wire safe attachment metadata loading through core task-scan APIs without invoking completion or reporting endpoints. Replaced by the transport/API subtask above.
  - [-] Surface attachment metadata in `tasks scan` output with golden JSON and offline CLI coverage. Replaced by the CLI/golden subtask above.
- [x] Add a structured `courses show` or equivalent command for a single course snapshot.
- [x] Add a read-only `exams show` or equivalent command for a single exam snapshot.
- [x] Expand fixture coverage to more blocked / error exam cover states.
- [x] Document the exact boundary between safe read-only endpoints and endpoints that may start timers or mutate state.

## Phase 3 — Execution Pipeline

- [x] Implement `CourseRunner`.
  - [x] Add core execution planning models plus deterministic `CourseRunPlan` building from scanned chapters, with unit tests.
  - [x] Add a planning-only `CourseRunner` that resolves a course plus task scan without executing side effects.
  - [x] Document current `CourseRunner` scope and unsupported module behavior until executor wiring lands.
  - [x] Preserve safe attachment metadata in `CourseRunPlan` so later executors can reuse read-only task snapshots without re-scanning.
  - [x] Add a flattened execution queue summary on top of `CourseRunPlan` so later executors can consume a stable task ordering contract.
  - [x] Add planning lifecycle `RunEvent` coverage for `CourseRunner` without introducing side-effecting task execution.
  - [x] Document the enriched `CourseRunner` planning contract and its handoff boundary to future executors.
- [x] Implement `ExamRunner`.
  - [x] Add core exam planning models plus deterministic `ExamRunPlan` building from read-only exam metadata and preview question inventory, with unit tests.
  - [x] Add a planning-only `ExamRunner` that resolves one exam snapshot plus preview inventory without calling start/submit exam endpoints.
  - [x] Add planning lifecycle `RunEvent` coverage for `ExamRunner` without introducing answer submission or timer mutations.
  - [x] Document `ExamRunner` planning scope, supported question kinds, and the preview-based safety boundary.
- [x] Implement `TaskExecutorRegistry`.
  - [x] Add a core `TaskExecutorRegistry` selector for known `execution_queue` modules, with fail-closed fallback for unsupported or inconsistent planning data, unit tests, and docs.
  - [x] Add `CourseRunPlan` helpers plus fixture-backed tests for ordered registry resolution across the flattened execution queue.
- [x] Keep runners UI-independent and emit `RunEvent` only.
  - [x] Replace the transport-layer `EventSink` with a shared core `RunEventSink` contract and rewire runners / CLI / transport to use it, with regression tests.
  - [x] Add explicit CLI-side run output adapters so collected `RunEvent`s stay outside `cpass-core`.
  - [x] Document the runner/UI boundary and future headless/TUI subscription model.
- [x] Implement a real `cpass run` headless execution path.
  - [x] Expose fail-closed executor preflight summaries for `CourseRunPlan.execution_queue` through `cpass run` output, with unit tests, golden JSON, and offline CLI coverage.
  - [x] Add runtime course-run event and result models for queue execution state transitions without widening the endpoint boundary.
  - [x] Add a headless queue driver that consumes the flattened execution queue in order and stops on unsupported or inconsistent planner output.
  - [x] Wire `cpass run` to the headless execution driver and document the new execution contract plus remaining module-specific gaps.
- [x] Add a thin TUI shell that only subscribes to events.
- [x] Port legacy execution support for:
  - [x] video task points
    - [x] Preserve legacy video execution metadata in read-only planning snapshots and add a validated core video execution context derived from `CourseRunQueueEntry`.
    - [x] Add legacy video play-report transport helpers, fixtures, and parser coverage for runtime progress acknowledgements.
    - [x] Implement a headless video queue executor that reuses the planned execution context, emits runtime `RunEvent`s, and completes fixture-backed video entries without widening support to other modules.
    - [x] Wire the video executor through `cpass run` output, goldens, and offline CLI integration tests while document/work entries remain fail-closed.
  - [x] document task points
    - [x] Preserve legacy document execution metadata in read-only planning snapshots and add a validated core document execution context derived from `CourseRunQueueEntry`.
    - [x] Add legacy document reading-report transport helpers, fixtures, and parser coverage for runtime acknowledgements.
    - [x] Implement a headless document queue executor that reuses the planned execution context, emits runtime `RunEvent`s, and completes fixture-backed document entries without widening support to other modules.
    - [x] Wire the document executor through `cpass run` output, goldens, and offline CLI integration tests while chapter-work entries remain fail-closed.
  - [x] chapter work task points
    - [x] Preserve legacy chapter-work execution metadata in read-only planning snapshots and add a validated core chapter-work execution context derived from `CourseRunQueueEntry`.
    - [x] Add legacy chapter-work page fixtures plus parser and transport coverage for runtime form discovery without submitting answers.
    - [x] Implement a fail-closed headless chapter-work executor that reuses the planned execution context, fetches runtime work snapshots, and stops before answer submission until answer resolution support lands.
    - [x] Wire the chapter-work planning/runtime snapshot through `cpass run` output, goldens, offline CLI integration tests, and docs.
- [x] Add a unified `SearcherProvider -> AnswerCandidate` pipeline.
  - [x] Add shared `AnswerQuery` / `AnswerQuestionKind` models plus a provider-ordered `SearcherPipeline`, with unit tests and docs.
  - [x] Add chapter-work runtime helpers that derive search queries from typed work snapshots without widening answer-submission support.
  - [x] Add executor-facing candidate selection scaffolding that keeps chapter-work execution fail-closed until concrete searchers and answer-mutation boundaries are ready.
- [x] Implement the first supported searcher set:
  - [x] JSON
  - [x] SQLite
  - [x] HTTP
    - [x] Add shared HTTP searcher request-template contracts for legacy form/query and JSON payload shapes, with unit tests and docs.
    - [x] Implement a fail-closed `HttpSearcherProvider` that validates config, issues requests, and extracts answer candidates from JSON responses.
    - [x] Wire `http` / legacy `restApiSearcher` / `JsonApiSearcher` through config docs, `cpass run`, and offline integration coverage.
  - [x] OpenAI-compatible
    - [x] Add shared OpenAI-compatible request/response contracts, prompt rendering helpers, unit tests, and contract docs.
    - [x] Implement a fail-closed `OpenAiCompatibleSearcherProvider` for chat-completions-style APIs, with backend abstraction tests.
    - [x] Wire `openai-compatible` / legacy `OpenAISearcher` through config docs, `cpass run`, and offline integration coverage.
- [x] Add stubbed integration tests for execution event ordering and export side effects.

## Phase 3.5 — Interactive Launcher Contract

- [x] Align the top-level `cpass` interactive launcher with the legacy `EXAM|...` course-to-exam routing contract without widening read-only safety boundaries.
  - [x] Route `EXAM|<course selector>` through the launcher, fetch the selected course exam list, and render an interactive exam picker without mutating exam state.
  - [x] Support exam selection / `e<index>` export in the launcher by reusing read-only `exam show` / `exam preview export` logic instead of starting or submitting an exam session.
  - [x] Add offline CLI integration tests and docs for the top-level read-only exam route.
- [x] Align the top-level `cpass` interactive launcher with the legacy error-surfacing contract so malformed live/fixture responses do not leak raw backend parse errors.
  - [x] Map interactive JSON/response parse failures to a legacy-style relogin hint instead of the raw `error: ...` prefix.
  - [x] Add offline CLI coverage and docs for the launcher-side error translation path.

## Phase 4 — Beyond Legacy

- [x] Add live task point support.
  - [x] Preserve raw chapter-card iframe `data` payload snapshots through the parser, `CourseRunPlan`, and `execution_queue` so future live executors can reuse legacy module data without re-scanning.
  - [x] Add parser models plus legacy fixtures and unit tests for live task metadata extracted from chapter-card snapshots and any safe read-only live context.
  - [x] Add validated live queue execution context and registry wiring that stays fail-closed until runtime endpoint contracts are verified.
    - [x] Add typed `live` task classification, executor-registry / preflight coverage, and explicit fail-closed CLI dispatch for fixture-backed `insertlive` queue entries.
    - [x] Add validated `LiveTaskExecutionContext` parsing from preserved iframe and attachment metadata without calling runtime endpoints.
    - [x] Document the registered-but-blocked live execution boundary before any runtime endpoint contracts are approved.
  - [-] Add fixture-backed `cpass run` coverage, golden baselines, and docs for the live task path. Replaced by the more specific typed live classification / fail-closed dispatch subtask above.
  - [x] Add live attachment parser fields, a legacy `knowledge/cards` fixture, and unit tests for raw `streamName` / `vdoid` / `liveId` descriptors captured from chapter-card attachment snapshots.
  - [x] Preserve live attachment descriptors in task-scan and planning snapshots without widening runtime support.
  - [-] Add a typed `live` task classification and executor-registry / preflight coverage once the chapter-card module mapping is fixture-backed. Replaced by the more specific typed live classification / fail-closed dispatch subtask above.
  - [x] Add legacy live progress transport helpers, fixtures, and parser coverage for runtime acknowledgements.
  - [x] Wire a fail-closed headless live executor through `cpass run`, golden outputs, offline CLI tests, and docs.
- [ ] Add article-reading task point support. Blocked: waiting for a real `insertbook`-style article fixture bundle plus reviewed runtime acknowledgement samples; the expanded 2026-04-12 audit across tracked files, git history, and local build/OMX artifacts still only found docs plus the placeholder `fixtures/legacy_article/README.md`.
  - [x] Document the missing legacy article-reading fixture prerequisite and define the minimum capture set required before parser or runtime work can proceed safely.
  - [x] Document the `fixtures/legacy_article` capture contract, including canonical chapter-card / attachment filenames and the still-open runtime acknowledgement route question.
  - [x] Audit the current repository fixture corpus and legacy reference surfaces, then record why article fixture capture remains externally blocked.
  - [x] Scaffold `fixtures/legacy_article/` with an in-repo placeholder README so the future external capture handoff has a tracked target path without inventing fake fixtures.
  - [x] Expand the in-repo article fixture audit across tracked files, git history, and local build/OMX artifacts, then record that there is still no real `insertbook` sample to canonicalize.
  - [ ] Capture a real article chapter-card JSON snapshot plus the matching `knowledge/cards` attachment HTML into `fixtures/legacy_article/`. Blocked: the expanded 2026-04-12 audit across tracked files, git history, and local build/OMX artifacts still only found docs plus the placeholder article bundle, so there is no safe in-repo `insertbook` sample to derive those payloads from.
  - [ ] Capture the reviewed article progress-report request/response pair and lock its eventual fixture filename + response format once a real article task sample exists. Blocked: the article runtime acknowledgement route is still unknown without that sample.
  - [ ] Add parser models and unit tests for article task metadata extracted from chapter-card snapshots and any safe read-only article context once fixture capture lands. Blocked: waiting for the real article chapter-card + attachment fixture bundle described in `docs/article-fixture-capture.md`.
  - [ ] Preserve article attachment metadata in task-scan and planning snapshots without approving runtime endpoints. Blocked: parser/planning work stays blocked until the real article chapter-card + attachment fixtures exist.
  - [ ] Add validated article queue execution context and fail-closed registry / CLI dispatch coverage until runtime endpoint contracts are reviewed. Blocked: runtime route review stays blocked until a real article progress-report request/response pair is captured.
  - [ ] Wire article runtime support, golden baselines, offline CLI tests, and docs once the article progress-report contract is fixture-backed and approved. Blocked: fixture-backed parser and runtime contracts do not exist yet for the article module family.
- [ ] Add richer question type support beyond single-choice, multiple-choice, true-false, and fill-in-the-blank. Blocked: richer parser fixture capture for non-classic layouts is still missing; the current audit plus the capture contract now live in `docs/rich-question-fixture-capture.md`.
  - [x] Extend shared question-kind normalization in exam planning and search query contracts for the legacy question-type ids already enumerated by the parser defaults, with unit tests and docs.
  - [x] Audit the current preview/work fixture corpus plus the legacy richer-question enum list, then document the missing richer-question capture contract before parser work resumes.
  - [ ] Capture a representative non-classic preview or chapter-work fixture bundle under `fixtures/legacy_rich_questions/` once a real richer-question sample exists. Blocked: the current in-repo fixtures only cover types `0/1/2/3` plus rich-option variants of type `0`, so there is still no real DOM sample for the richer legacy ids listed in `cxapi/schema.py`.
  - [ ] Add parser fixtures and unit tests for representative richer question layouts once the `fixtures/legacy_rich_questions` bundle exists. Blocked: parser coverage must wait for the real richer-question DOM samples described in `docs/rich-question-fixture-capture.md`.
  - [-] Add parser fixtures and unit tests for representative richer question layouts once real preview or work samples are captured. Replaced by the more specific audit/capture/parser subtasks above.
  - [x] Surface richer question-kind metadata through run/search outputs, golden baselines, and offline coverage for the supported fixture paths.
  - [x] Document the remaining unsupported rich-text or media-heavy question structures and the fail-closed executor boundary.
- [x] Add support for rich-text or image-based options where feasible.
  - [x] Preserve structured rich-text / image option metadata in exam preview and chapter-work parser snapshots, with unit tests and contract docs.
  - [x] Add fixture-backed `exam preview export` coverage plus a golden baseline for rich option metadata.
  - [x] Add fixture-backed `cpass run` chapter-work snapshot coverage plus a golden baseline for rich option metadata.
  - [x] Extend searcher-facing query rendering and docs to consume preserved rich option metadata without claiming OCR or media understanding.
  - [-] Add fixture-backed export / run coverage plus golden baselines for rich option metadata once representative fixtures exist. Replaced by the more specific preview-export and chapter-work runtime coverage subtasks above.
- [x] Add notification channels such as Gotify or MQTT.
  - [x] Define notification channel config models, validation, and docs for Gotify/MQTT without sending messages yet.
  - [x] Add CLI-owned notification sink plumbing that can summarize lifecycle `RunEvent`s without changing core runner boundaries.
  - [x] Implement a Gotify notification backend with fixture-backed tests and docs.
  - [x] Implement an MQTT notification backend with fixture-backed tests and docs.
    - [x] Add CLI-owned MQTT notifier config parsing, summary payload contracts, and fixture replay coverage with unit tests.
    - [x] Wire fixture-backed MQTT delivery through `dispatch_notification_plans`, offline CLI coverage, and notification docs without widening runner boundaries.
    - [x] Document the live MQTT transport/TLS approval gate, including the client-dependency acceptance criteria that must be satisfied before fixture-only delivery can grow into a real broker backend.
    - [-] Add live broker delivery for supported MQTT transports once an explicit runtime transport / TLS dependency approach is approved. Replaced by the more specific dependency-evaluation / live-backend / verification subtasks below.
    - [x] Compare candidate live MQTT transport/TLS dependency approaches against the current CLI acceptance criteria and record the recommended path in `docs/mqtt-live-delivery-evaluation.md`.
    - [x] Approve a CLI-only `rumqttc` + Rustls live MQTT dependency package and record the rollout guardrails in `docs/mqtt-live-delivery-approval.md`.
    - [x] Add a live `MqttBackend` implementation for the approved runtime dependency while keeping fixture replay as the deterministic offline path.
      - [x] Extend MQTT notifier config parsing and backend-selection scaffolding so fixture replay and future live delivery share one validated config path, with unit tests and docs.
      - [x] Verify the offline automation environment still lacks a cached `rumqttc` source tree and record that blocker in the live MQTT rollout docs/checklist before retrying implementation work.
      - [-] Add offline CLI coverage plus notification docs for the current live-MQTT warning path so missing `fixture_response_path` remains a verified pre-live state while `rumqttc` stays unavailable locally. Replaced by the landed live-backend CLI coverage + live-delivery docs once the one-shot broker path shipped.
      - [-] Implement the approved live broker publish/disconnect backend (`rumqttc` + Rustls) once the dependency source is available locally. Replaced by the landed CLI-owned one-shot MQTT publish/disconnect backend built from already-cached Tokio TCP + Rustls workspace crates.
      - [x] Wire the live backend through `dispatch_notification_plans` documentation while keeping fixture replay as the deterministic offline coverage path.
      - [x] Restore the live MQTT one-shot backend protocol helpers so the checked-in `mqtt://` / `mqtts://` broker-backed CLI validation coverage compiles again.
      - [x] Add deterministic in-memory live MQTT validation in `crates/cpass-cli/src/notification.rs` for plain/TLS publish flows plus extra-root trust loading so the backend is still exercised when loopback listeners are unavailable.
      - [x] Re-run the ignored local `mqtt://` / `mqtts://` broker-backed CLI validation in an environment that permits loopback listeners, and record the 2026-04-12 passing plain/TLS reruns after normalizing the JSON payload assertion in `crates/cpass-cli/tests/offline_cli.rs`.
    - [-] Record the landed in-repo plain-`mqtt://` broker harness (`config_validate_dispatches_live_mqtt_notification_to_local_broker`) and narrow the remaining validation gap to TLS broker coverage in docs/checklist. Replaced by the more specific restored broker-backed validation subtask above.
    - [-] Add live MQTT delivery validation coverage and final docs once the live backend exists and a broker-backed verification strategy is approved. Replaced by the restored compile-validating subtask above plus the completed broker-validation rerun.
- [x] Add non-interactive profiles and stronger automation-oriented CLI flows.

## Phase 5 — Mutating Exam Parity

- [ ] Restore the legacy mutating exam worker semantics (`get_meta` → `start` → fetch/save answers → final submit) in the Rust launcher. Blocked: the repository still only has read-only exam cover/preview fixtures; there is no reviewed fixture bundle for `exam/phone/start`, `exam/test/reVersionTestStartNew`, `exam/phone/loadAnswerStatic`, or `exam/test/reVersionSubmitTestNew`, so implementing side-effecting exam flows would violate the current approved-contract rule.
  - [x] Audit the legacy mutating exam flow and record the exact endpoint/state prerequisites plus the fixture capture contract required before implementation can begin safely.
  - [ ] Capture one reviewed mutating exam session fixture bundle covering cover metadata, start redirect, answer-sheet status, question fetch, preview refresh, per-question submit/save, and final-submit responses. Blocked: no such reviewed runtime corpus exists in-repo yet.
  - [ ] Add core exam-session parser/runtime models for mutable exam state (`enc`, remain-time fields, answer-sheet state, submit acknowledgements) once the reviewed fixture bundle exists. Blocked: parser/runtime contracts depend on the missing reviewed mutating exam fixtures.
  - [ ] Implement the launcher exam worker semantics, offline CLI coverage, and docs once the mutating exam fixture bundle is reviewed and approved. Blocked: the required start/submit fixture-backed runtime contract has not been approved.
  - [x] Add named config profiles with partial override merge rules plus `--profile` / `CPASS_PROFILE` selection, unit tests, offline CLI coverage, and docs.
  - [x] Surface the selected profile and normalized automation paths in `config validate` / `doctor` output for scripting-friendly inspection.
  - [x] Add stricter non-interactive session and target-resolution helpers for automation-oriented CLI flows, with offline coverage.
  - [x] Document profile-driven automation recipes and the remaining interactive boundaries for `login`, `run`, and export commands.
- [x] Promote the Rust path to the only documented default runtime.
- [x] Keep the Python path only for fixture capture and protocol comparison.
  - [x] Document the allowed legacy Python workflows in README/docs, including when Poetry may still be used for fixture capture or protocol comparison.
  - [x] Mark the Python environment metadata and examples as legacy-only so they no longer read like a supported runtime.
  - [x] Audit the remaining repository guidance so Python-side diffs stay limited to fixture capture or protocol comparison handoffs, with the canonical checkpoints kept in README + `docs/legacy-reference.md`.

## Open Technical Notes

- The fixture transport currently supports:
  - course list
  - chapter list
  - chapter status
  - chapter cards
  - chapter card attachment snapshots
  - chapter work page discovery
  - video attachment status
  - video play-report acknowledgements
  - document reading-report acknowledgements
  - live progress acknowledgements
  - exam list
  - account info
  - exam cover metadata
  - exam preview question inventory
- `exam export` is read-only and currently limited to catalog plus cover metadata.
- `exam show` is read-only and currently limited to a single catalog entry plus cover metadata.
- Exam cover fixtures now cover completed redirects plus blocked/error cover pages for not-started, unfinished-chapter, IP-restricted, and PC-client-only states.
- `tasks scan` is read-only and currently limited to chapter progress plus task summary extraction from card iframes, `knowledge/cards` attachment snapshots, and `ananas/status/{objectId}` video metadata.
- Attachment metadata work currently treats only `GET /knowledge/cards` and `GET /ananas/status/{objectId}` as safe read-only sources.
- `cpass-core` can now fixture-load chapter-work runtime discovery snapshots by combining `GET /knowledge/cards` work attachment tokens with `GET /android/mworkspecial`, and the new headless chapter-work executor emits that typed snapshot before it still stops fail-closed ahead of any work save/submit route.
- `cpass run` now reuses the fixture-backed `/multimedia/log/a/{cpi}/{dtoken}` acknowledgement path for planned video entries, the fixture-backed `/ananas/job/document` acknowledgement path for planned document entries, and the fixture-backed chapter-work discovery path to expose typed runtime snapshots before it still stops fail-closed ahead of answer mutation or any later unsupported runtime module.
- `cpass-core` now also exposes the fixture-backed `https://zhibo.chaoxing.com/saveTimePc` live progress acknowledgement contract plus typed parsing for legacy live task runtime groundwork, but the public CLI still blocks fail-closed before dispatch can call it.
- `TaskExecutorRegistry` now classifies `insertlive` queue entries as the typed `live` module family and `cpass run` exposes them as registered preflight entries, but the CLI still blocks fail-closed before any live runtime endpoint is called.
- Article-reading support is currently blocked on capturing a legacy chapter-card sample plus matching attachment/runtime payloads for the article module family; this repository does not yet contain any `insertbook`-style fixture or a legacy Python implementation that could safely define those contracts.
- The planned `fixtures/legacy_article` root plus chapter-card / attachment naming contract now lives in `docs/article-fixture-capture.md`; do not add fixture transport mappings or runtime route assumptions until a real article task sample is captured.
- Richer question parser work is currently blocked on capturing a real non-classic preview or chapter-work DOM sample; `docs/rich-question-fixture-capture.md` records that the checked-in fixtures still only prove types `0/1/2/3` plus type-`0` rich-option variants, even though the legacy enum lists broader ids.
- The MQTT live-delivery dependency comparison now lives in `docs/mqtt-live-delivery-evaluation.md`, and the landed CLI-only rollout guardrails live in `docs/mqtt-live-delivery-approval.md`; fixture replay remains the deterministic offline path, `crates/cpass-cli/src/notification.rs` has in-memory plain/TLS validation coverage for the live backend, and the ignored broker-backed harnesses for `mqtt://` / `mqtts://` were rerun successfully in a listener-capable environment on 2026-04-12.
- The repository now has deterministic in-memory live-MQTT validation in `crates/cpass-cli/src/notification.rs` (`live_mqtt_backend_publishes_over_plain_duplex_session`, `live_mqtt_backend_publishes_over_tls_duplex_session`, and `load_mqtt_root_store_adds_extra_root_certificate_from_env`), plus ignored broker-backed follow-up harnesses in `crates/cpass-cli/tests/offline_cli.rs` (`config_validate_dispatches_live_mqtt_notification_to_local_broker` and `config_validate_dispatches_live_mqtts_notification_to_local_tls_broker`). The TLS path is rooted in the repo-tracked fixture bundle under `crates/cpass-cli/tests/fixtures/mqtt_tls/` plus `CPASS_MQTT_EXTRA_ROOT_CERT_DER`; the harnesses stay opt-in because they require loopback listeners, but both plain/TLS reruns passed on 2026-04-12 after the payload assertion was made order-insensitive.
- Attachment progress/reporting routes such as `/multimedia/log/a` and `/ananas/job/document` remain out of scope for read-only commands; Phase 3 runtime work may only add them behind explicit fixture-backed executor tasks.
- Any work that touches exam preview or answer submission must be reviewed against the “safe read-only” boundary before implementation.
- The Phase 2 read-only allowlist and denylist are documented in `docs/read-only-boundary.md`.
