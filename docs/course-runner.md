# CourseRunner

`CourseRunner` is the current Phase 3 execution bootstrap in `cpass-rs`.

`CourseRunner` itself is still intentionally limited to planning. It resolves one course, reuses the existing read-only task-scan pipeline, emits planning lifecycle `RunEvent`s, and builds a deterministic `CourseRunPlan` snapshot for the CLI.

`cpass-core` now contains `HeadlessVideoCourseRunExecutor`, `HeadlessDocumentCourseRunExecutor`, and a fail-closed `HeadlessChapterWorkCourseRunExecutor` as task-specific runtime building blocks. The public `cpass run` command still layers `CourseRunHeadlessDriver` on top of the plan, keeps presentation outside `cpass-core`, and now wires all three executors so `chapter_work` entries can expose a typed runtime snapshot before they still stop fail-closed ahead of any answer mutation.

Implementation entrypoints:

- `crates/cpass-core/src/execution.rs`: `CourseRunner`, `CourseRunPlan`, and task-kind mapping
- `crates/cpass-core/src/event.rs`: `RunEvent` plus the `RunEventSink` contract used by runners
- `crates/cpass-cli/src/run_output.rs`: CLI-owned event buffering and JSON output adapters for `cpass run --json`
- `crates/cpass-cli/src/run_tui.rs`: CLI-owned thin TUI shell that subscribes to `RunEvent`
- `crates/cpass-cli/src/main.rs`: `cpass run` command wiring

## Current Scope

The current runner does exactly four things:

1. Emit `course_run_planning_started` with the requested `--course-id` or `--course-index` selector.
2. Resolve the target course from `--course-id` or `--course-index`.
3. Fetch the same read-only chapter/task snapshot used by `tasks scan`.
4. Convert that snapshot into a stable `CourseRunPlan` ordered by chapter index, card index, point index, module, and resource id, then mirror that same order into a flattened `execution_queue`, followed by `course_run_planning_finished` with the resolved course id plus chapter/task-point counts.

The result is a planning artifact only. It is safe to inspect, save, diff, or use as future executor input, but it is not an execution engine yet.

## Current Data Contract

`CourseRunPlan` currently includes:

- the resolved course identity and metadata
- one `CourseRunChapterPlan` per scanned chapter
- one flattened `execution_queue` entry per scanned task point, in the same stable order as the chapter/task snapshot
- a flattened `total_task_points` count
- one `CourseRunTaskPoint` per scanned task point with:
  - `card_index`
  - `point_index`
  - raw `module`
  - normalized `task_kind`
  - `title`
  - `resource_id`
  - optional raw `iframe_data`, copied from the chapter-card `<iframe data='...'>` snapshot so later module-specific executors can reuse legacy payloads without re-scanning
  - optional `attachment_metadata` copied from the safe read-only task-scan path when available

Each `execution_queue` entry repeats the execution-relevant fields plus chapter context:

- `queue_index`
- `chapter_id`
- `chapter_index`
- `chapter_name`
- `chapter_label`
- the task-point identity fields already listed above

That queue is the new handoff boundary for later executors. Future task handlers can consume a single stable ordering contract without having to re-flatten the nested chapter structure themselves.

The runner preserves the raw module string even when it can classify the task. It also preserves the safe attachment snapshot already collected by `tasks scan`, such as document descriptors, video duration/status metadata, or raw live attachment descriptors captured from `knowledge/cards`, in both the nested chapter view and the flattened queue view. That keeps the planning output stable for future executor work, avoids hiding unsupported modules, and lets later executors reuse the read-only planning snapshot without re-scanning chapters first.

## Executor Handoff Boundary

The planner now exposes two views over the same snapshot, but they serve different consumers:

- `chapters[*].task_points` is the human-readable course outline for CLI inspection, debugging, and future UI rendering.
- `execution_queue[*]` is the machine-oriented executor handoff surface for `TaskExecutorRegistry`, headless `cpass run`, and later UI shells.
- `execution_preflight.queue[*]` is the CLI-facing fail-closed registry summary keyed by `execution_queue[*].queue_index`, so operators can see whether each planned task point maps to a known executor key before any runtime dispatch exists.
- `CourseRunExecutionResult.queue[*]` is the future runtime snapshot surface for the headless queue driver, keyed by the same `queue_index` and carrying both the fail-closed registry resolution plus a runtime state (`pending`, `running`, `completed`, or `blocked`).

The current registry bootstrap is documented in [`docs/task-executor-registry.md`](./task-executor-registry.md). Today that registry can validate individual queue entries and walk an entire planned queue in stable order, but it still does not execute anything.

`CourseRunHeadlessDriver` is the first core-only consumer of that queue contract. It accepts a `CourseRunPlan`, resolves each entry through `TaskExecutorRegistry`, delegates registered entries to an injected `CourseRunQueueEntryExecutor`, and stops immediately if the planner output is unsupported, inconsistent, or the injected executor reports a blocked entry. That keeps the ordered queue walk and fail-closed runtime bookkeeping centralized in `cpass-core` without pushing execution-state transitions into the CLI.

Future executor work should treat the flattened queue as authoritative for execution order and task identity. The current contract is:

- `queue_index` is a dense zero-based sequence derived from the planner sort order. Executors should consume entries in ascending `queue_index` instead of re-sorting the nested chapter view.
- every queue entry is a self-contained copy of one planned task point plus the chapter context needed to label progress or correlate side effects back to the course tree.
- `task_kind` is a convenience classification for known module families, but `module` remains the lossless source field. The current planner normalizes `insertvideo`, `insertdoc`, `insertlive`, and `work`; unsupported modules must stay visible as `unknown(...)` instead of being dropped during planning.
- `iframe_data` is the lossless chapter-card payload snapshot captured during planning. Future module-specific executors such as live-task support can parse it later without re-fetching `knowledge/cards`, and current CLI/fixture outputs keep that snapshot visible for fixture capture.
- `attachment_metadata` is an optional safe read-only snapshot inherited from `tasks scan`. Executors may reuse it as cached input, but they must tolerate `null` and must not widen the endpoint boundary just to fill missing metadata. For unsupported module families such as fixture-backed live tasks, this snapshot is currently limited to preserved attachment descriptors only and must not be treated as runtime support.
- `total_task_points == execution_queue.len()` and `course_run_planning_finished.task_points` currently reports that same flattened count. Consumers can use that invariant for progress baselines without re-counting chapters.

The first module-specific runtime handoff is now defined for video queue entries. `CourseRunQueueEntry::video_execution_context()` validates that the planned entry is still an `insertvideo` task point and that the preserved read-only metadata includes the legacy runtime fields needed for play-report requests: `fid`, `job_id`, `other_info`, `duration_secs`, `dtoken`, and a playback rate (preserved from attachment `rt` when present, otherwise defaulted to the legacy `0.9`).

`insertdoc` entries now expose the next planning/runtime handoff as well. `CourseRunQueueEntry::document_execution_context()` validates that the preserved attachment snapshot still carries the legacy document runtime fields needed for `/ananas/job/document`: `job_id`, `jtoken`, and the attachment `object_id` tied back to the planned `resource_id`. `DocumentTaskExecutionContext::build_reading_report_request(...)` now turns that validated queue entry into the legacy reading-report query contract, and `HeadlessDocumentCourseRunExecutor` now participates in the CLI runtime stack alongside the video executor.

`chapter_work` entries now have the first runtime discovery scaffold even though they still do not submit or resolve answers. `ChaoxingClient::fetch_chapter_work_form(...)` now replays the legacy two-step fetch sequence inside `cpass-core`: it first re-reads the rendered `knowledge/cards` attachment page to recover the work-specific `ktoken` and `enc` tokens, then it loads `/android/mworkspecial` and normalizes the resulting hidden form fields plus question inventory into a typed `ChapterWorkFormSnapshot`. `ChapterWorkTaskExecutionContext::build_runtime_request(...)` converts the planned queue entry plus resolved course/account context into that fetch contract, and `HeadlessChapterWorkCourseRunExecutor` now consumes it to emit a typed `course_run_chapter_work_snapshot_fetched` event before stopping fail-closed. When an optional `SearcherPipeline` is injected, the same executor can now also derive provider-ordered candidate selections from that snapshot and emit a summary-level `course_run_chapter_work_candidate_selection_prepared` event; the public CLI now wires the first local backends (`json` / legacy `jsonFileSearcher`, `sqlite` / legacy `SqliteSearcher`) into that path. Even so, it still stops before any answer save or submit route is called. That keeps the current runtime boundary explicit: fetching the work page and preparing search-only candidate selections are now fixture-backed and typed, but anything that would save, submit, resolve, or mutate the work attempt still belongs to later executor tasks.

`insertlive` entries are now classified as the typed `live` module family, so `execution_preflight` can report them as registered instead of collapsing them into `unknown(...)`. That registration still does not imply open-ended runtime support. The public CLI now hands registered `live` queue entries to a dedicated `HeadlessLiveCourseRunExecutor`, but that executor is intentionally narrow: it reuses the validated planned metadata to submit exactly one reviewed `saveTimePc` acknowledgement attempt, emits a typed runtime event, and then still blocks the queue before any attendance, completion, or additional live runtime route is called.

`insertlive` entries now also expose `CourseRunQueueEntry::live_execution_context()` as a typed handoff built entirely from preserved planning snapshots. That validator requires the chapter-card `iframe_data` payload plus the safe `knowledge/cards` attachment snapshot to agree on `liveId`, `vdoid`, and `streamName`, and it also requires the attachment `job_id` needed for the reviewed acknowledgement contract. `LiveTaskExecutionContext::build_progress_report_request(...)` turns that validated queue entry into the legacy `https://zhibo.chaoxing.com/saveTimePc` query payload, and `ChaoxingClient::report_live_progress(...)` plus `parse_live_progress_report_ack(...)` keep that contract fixture-backed inside `cpass-core`. `HeadlessLiveCourseRunExecutor` now consumes that path, emits `course_run_live_progress_reported`, and then still stops fail-closed after the first reviewed acknowledgement attempt instead of claiming the live task is complete.

The exact "registered but blocked" contract for live task points is documented in [`docs/live-task-boundary.md`](./live-task-boundary.md). Treat that document as the approval gate until a later checklist item explicitly lands fixture-backed live runtime endpoint support.

Those handoffs now have concrete transport consumers inside `cpass-core`:

- `VideoTaskExecutionContext::build_play_report_request(...)` produces the legacy `/multimedia/log/a/{cpi}/{dtoken}` payload, including the legacy `enc` signature.
- `ChaoxingClient::report_video_progress(...)` maps that payload onto the transport layer and `parse_video_play_report_ack(...)` normalizes the runtime acknowledgement into a typed `VideoPlayReportAck`.
- `DocumentTaskExecutionContext::build_reading_report_request(...)` produces the legacy `/ananas/job/document` query payload, `ChaoxingClient::report_document_progress(...)` maps it onto the transport layer, and `parse_document_reading_report_ack(...)` normalizes the acknowledgement into a typed `DocumentReadingReportAck`.
- `LiveTaskExecutionContext::build_progress_report_request(...)` produces the legacy `https://zhibo.chaoxing.com/saveTimePc` query payload, and `ChaoxingClient::report_live_progress(...)` plus `parse_live_progress_report_ack(...)` keep the first reviewed live acknowledgement route fixture-backed under the public executor boundary.
- `HeadlessVideoCourseRunExecutor` reuses the validated queue entry plus the resolved course/account context, submits one completion-shaped play-report acknowledgement for fixture-backed video entries, emits `course_run_video_progress_reported`, and only marks the queue entry completed when the acknowledgement returns `isPassed = true`.
- `HeadlessDocumentCourseRunExecutor` reuses the validated queue entry plus the resolved course context, submits one legacy reading-report acknowledgement for fixture-backed document entries, emits `course_run_document_progress_reported`, and only marks the queue entry completed when the acknowledgement returns `success = true`.
- `HeadlessLiveCourseRunExecutor` reuses the validated queue entry plus the resolved course/account context, submits one reviewed `saveTimePc` acknowledgement for fixture-backed live entries, emits `course_run_live_progress_reported`, and then immediately stops fail-closed before it can infer attendance, completion, or approval for any additional live runtime route.
- `HeadlessChapterWorkCourseRunExecutor` reuses the validated queue entry plus the resolved course/account context, fetches one typed runtime work snapshot through the existing fixture-backed discovery path, can optionally prepare provider-ordered candidate selections from that snapshot, emits `course_run_chapter_work_snapshot_fetched` plus the summary `course_run_chapter_work_candidate_selection_prepared` event when a searcher pipeline is injected, and then immediately stops fail-closed before any answer save or submit endpoint is called.
- The public CLI path now hands `chapter_work` entries to that executor and can instantiate the supported JSON, SQLite, HTTP, and OpenAI-compatible searchers from config (including the legacy aliases noted in `docs/searcher-pipeline.md`), so `cpass run --json` / `--tui` can expose the typed runtime snapshot event plus an optional candidate-selection summary without widening support to answer save/submit routes.

Just as importantly, the planner stops at the queue boundary. Anything after that belongs to future executor layers, not to `CourseRunner` itself:

- extending runtime support beyond the current live/chapter-work fail-closed paths to answer resolution, answer submission, or future module kinds
- deciding retry cadence, multi-heartbeat playback behavior, or backoff policy for individual queue entries
- widening runtime support beyond the current single-ack video/document executor paths
- moving any task-execution, reporting, or submission endpoint call into `CourseRunner` itself instead of keeping it behind executor layers

That separation keeps `CourseRunner` reusable as a deterministic planning stage. Later executors can accept a serialized `CourseRunPlan` as input, consume the queue in order, and add runtime behavior without changing how the safe read-only snapshot is produced.

## Runtime State Scaffolding

Phase 3 now also defines the core-only runtime contract that the headless queue driver and executor stack use:

- `CourseRunExecutionResult`: one execution-level snapshot keyed by `course_id`, with aggregate counts and an overall runtime state (`pending`, `running`, `completed`, or `blocked`)
- `CourseRunQueueExecutionResult`: one per `execution_queue[*]` entry, preserving the fail-closed `TaskExecutorResolution` from preflight alongside the runtime queue state
- `RunEvent` variants `course_run_execution_started`, `course_run_queue_entry_state_changed`, `course_run_video_progress_reported`, `course_run_document_progress_reported`, `course_run_live_progress_reported`, `course_run_chapter_work_snapshot_fetched`, and `course_run_execution_finished` so a CLI or the thin TUI shell can observe queue transitions plus acknowledged runtime progress without depending on UI-specific types

These models are still presentation-free. They do not depend on CLI/TUI output concerns, and they keep the runtime state vocabulary stable while executor support expands from the current video-plus-document path to later modules.

The headless driver now sits on top of those same models. It is still executor-agnostic: the driver knows how to walk the queue, preserve ordering, stop fail-closed, and emit runtime events, but it does not embed module-specific Chaoxing behavior by itself.

## CLI Headless Execution Contract

`CourseRunner` still stops at planning, but the CLI now immediately feeds the resulting plan into `CourseRunHeadlessDriver` using a mixed video/document/chapter-work executor stack.

That means `cpass run --json` now returns three closely related runtime surfaces:

- `execution_preflight`, the registry-derived summary for the whole flattened queue before runtime dispatch starts
- `execution_result`, the runtime snapshot after the headless queue driver starts consuming entries in order
- `events`, the buffered planning and runtime lifecycle emitted by the planner, the driver, and the current mixed video-plus-document-plus-chapter-work executor stack

The current CLI executor stack intentionally has a narrow runtime envelope. It completes `insertvideo` entries by sending one completion-shaped legacy play-report acknowledgement, completes supported `insertdoc` entries through one legacy reading-report acknowledgement, and fetches one typed runtime work snapshot for `chapter_work` entries before immediately stopping fail-closed ahead of any answer save or submit route. That means `chapter_work` still blocks the public queue after any earlier supported video/document entries finish, but the public CLI now exposes the typed runtime discovery surface instead of stopping at raw dispatch bookkeeping.

That gives operators a stable runtime contract now:

- `execution_result.state` tells whether the queue completed or stopped fail-closed
- `execution_result.queue[*].state` mirrors the driver walk per `queue_index`
- `pending` means the driver never reached that entry because an earlier entry blocked
- `blocked` on a `chapter_work` entry currently means the queue either emitted one typed runtime work snapshot and then stopped fail-closed or the runtime snapshot fetch itself failed and was surfaced as a warning before any answer mutation route was called

## RunEvent Coverage

When the CLI wires its `RunEventBuffer` into `CourseRunner`, `CourseRunHeadlessDriver`, and the current executor stack, `cpass run --json` now returns the plan plus:

- `execution_preflight`, a registry-derived summary with:
  - `all_entries_registered`
  - `total_entries`
  - `registered_entries`
  - `blocked_entries`
  - one `queue[*]` item per `execution_queue[*]` entry, keyed by `queue_index` and tagged with the fail-closed `TaskExecutorResolution`
- `execution_result`, a runtime snapshot with:
  - overall `state`
  - `total_entries`
  - `completed_entries`
  - `blocked_entries`
  - one `queue[*]` item per `execution_queue[*]` entry, keyed by `queue_index` and carrying both the fail-closed `TaskExecutorResolution` plus the runtime queue state
- an `events` array that records the planning and runtime lifecycle:

- `course_run_planning_started`: emitted before course resolution and task scanning, with the caller's `course_id` / `course_index` selector
- `course_run_planning_finished`: emitted after the plan is built, with the resolved `course_id`, chapter count, and flattened task-point count
- `course_run_execution_started`: emitted before the headless queue driver starts walking the flattened queue
- `course_run_queue_entry_state_changed`: emitted whenever the active queue entry moves to `running`, `completed`, or `blocked`
- `course_run_video_progress_reported`: emitted by `HeadlessVideoCourseRunExecutor` after a video play-report acknowledgement is parsed for the active queue entry
- `course_run_document_progress_reported`: emitted by `HeadlessDocumentCourseRunExecutor` after a document reading-report acknowledgement is parsed for the active queue entry
- `course_run_chapter_work_snapshot_fetched`: emitted by `HeadlessChapterWorkCourseRunExecutor` after the typed runtime work-page snapshot is loaded for the active queue entry
- `warning`: emitted when a runtime acknowledgement or chapter-work snapshot fetch cannot be trusted or when the queue intentionally stops fail-closed before unsupported answer mutation begins
- `course_run_execution_finished`: emitted when the headless queue driver stops, with aggregate runtime counts and the final execution state

`execution_preflight.all_entries_registered == true` means the current queue can be fully classified by `TaskExecutorRegistry`. It does not mean every registered module has full public CLI execution support; today `video` and `document` execute their legacy acknowledgement paths on `cpass run`, while `chapter_work` currently stops after one typed runtime snapshot (or a fail-closed snapshot-fetch warning) instead of continuing into answer resolution or submission.

The current CLI path now proves slightly more than dispatch bookkeeping: it can drive the queue deterministically, acknowledge supported video entries through the legacy play-report route, acknowledge supported document entries through the legacy reading-report route, emit runtime `RunEvent`s for both acknowledgement paths, and still stop fail-closed once it reaches unsupported modules.

## UI Boundary

`CourseRunner` stays UI-independent on purpose. Its core surface is just:

- `build_plan(...) -> Result<CourseRunPlan>`
- optional `with_sink(Arc<dyn RunEventSink>)` wiring so callers can observe `RunEvent`

It does not print to stdout, render terminal widgets, own progress bars, or depend on CLI/TUI-specific types. The CLI is only one consumer: it owns both the JSON event buffer adapter and the thin `run --tui` shell, serializes or renders the returned state, and decides how to present that state. The current TUI shell intentionally stays thin: it subscribes to `RunEvent`, derives its own queue/progress snapshot from those events, and does not reach back into `cpass-core` for UI-only state. Future shells should keep the same boundary instead of pushing presentation concerns into `cpass-core`.

`cpass run --tui` uses the same planner, registry, headless driver, and current mixed video-plus-document-plus-chapter-work executor path as `cpass run --json`. The only difference is presentation:

- `--json` keeps the existing buffered `events` array plus serialized plan/runtime payload
- `--tui` swaps in a CLI-owned `RunEventSink` that renders planning status, queue states, warnings, and recent events
- when stdout is not a terminal, the TUI falls back to a single deterministic text snapshot so captured logs remain stable
- `--tui` is intentionally incompatible with `--json`

## Module Classification

The planner currently recognizes only the module names already exposed by Phase 2 task scanning:

| Raw module | Planned `task_kind` | Current meaning |
| --- | --- | --- |
| `insertvideo` | `video` | Video task point discovered from chapter-card snapshots |
| `insertdoc` | `document` | Document task point discovered from chapter-card snapshots |
| `work` | `chapter_work` | Chapter work / quiz task point discovered from chapter-card snapshots |
| any other value | `unknown(<module>)` | Preserved for later executor support instead of being dropped |

Unknown modules are not treated as errors during planning. They remain visible in the plan so fixture capture, parser work, and later executor wiring can extend support without changing the planner contract.

## Unsupported Behavior

Even with the current core executor paths, `CourseRunner` itself does not:

- start playback for video task points
- simulate multi-heartbeat video playback
- open chapter work pages or submit chapter work answers
- call any task-completion, reporting, timer, or submission endpoint
- skip, retry, or recover individual modules based on execution outcomes

This means `CourseRunner` remains closer to `tasks scan + normalization` than to the old legacy
capture/reference scripts. The core runtime layer now has narrow video/document acknowledgement
executors plus a public fail-closed chapter-work snapshot executor, and the CLI stops once
`chapter_work` reaches the answer-mutation boundary.

`video` and `document` now have public executor paths through `cpass run`, while `chapter_work` now exposes one typed runtime work snapshot event on the public CLI path and then still stops before any answer mutation route.

## Safety Boundary

The current runner inherits the same safe inputs as the read-only task scan path:

- course catalog resolution
- chapter tree loading
- chapter status snapshot loading
- chapter card parsing
- read-only attachment metadata snapshots already approved for `tasks scan`

It must not add any new endpoint beyond that boundary until the new route is reviewed, fixture-backed, and documented. See [`docs/read-only-boundary.md`](./read-only-boundary.md) for the current allowlist and denylist.

## Operator Expectations

When using `cpass run` today, expect:

- deterministic JSON output for the same fixture set
- visibility into which task points the runner sees
- explicit `unknown` task kinds when a module has no typed mapping yet
- a runtime `execution_result` snapshot and runtime events from the headless queue driver
- completion-shaped legacy play-report and reading-report acknowledgements for supported video/document entries before later `chapter_work` answer-mutation boundaries block
- typed `course_run_chapter_work_snapshot_fetched` events when fixture-backed chapter-work runtime discovery succeeds

Do not expect:

- successful task execution for `chapter_work` through the public CLI path yet
- multi-heartbeat or long-running video playback simulation
- concrete answer lookup backends or submission
- background progress updates
- parity with every legacy capture or comparison workflow

Those behaviors belong to the remaining Phase 3 items such as module-specific executor ports, the unified searcher pipeline, and richer event-driven UI wiring. New executor work should extend the current contract additively where possible so the queue handoff surface remains stable for fixtures, golden outputs, and downstream consumers.
