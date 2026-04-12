# Live Task Runtime Boundary

This document defines the current Phase 4 boundary for `insertlive` task points in `cpass-rs`.

The short version is intentional:

- live task points are now recognized, preserved, and typed during planning
- the core transport layer can now replay fixture-backed legacy `saveTimePc` progress acknowledgements
- the executor registry reports them as `registered`
- `cpass run` now attempts one reviewed `saveTimePc` acknowledgement and still blocks fail-closed before it can infer live completion

That distinction matters. A registered queue entry is only a stable planner-to-executor contract. The new single-ack executor path is not blanket approval to start open-ended live playback, repeated heartbeat loops, attendance confirmation, or completion flows.

## What Is Supported Today

The current live-task path is limited to cached planning data that was already collected from safe snapshots:

- chapter-card `<iframe data='...'>` payloads preserve raw `liveId`, `vdoid`, and `streamName`
- `knowledge/cards` attachment snapshots preserve the matching live attachment descriptor plus the legacy `job_id`
- `CourseRunPlan` and `execution_queue` keep `insertlive` entries visible as typed `live` task points instead of collapsing them into `unknown(...)`
- `TaskExecutorRegistry` resolves those queue entries as `registered` under the `live` executor key
- `CourseRunQueueEntry::live_execution_context()` validates that the preserved iframe payload and attachment snapshot agree on `liveId`, `vdoid`, `streamName`, and `job_id`
- `LiveTaskExecutionContext::build_progress_report_request(...)` and `ChaoxingClient::report_live_progress(...)` now encapsulate the legacy `https://zhibo.chaoxing.com/saveTimePc` acknowledgement contract under fixture coverage
- `parse_live_progress_report_ack(...)` normalizes the returned acknowledgement into a typed `LiveProgressReportAck`
- `HeadlessLiveCourseRunExecutor` now reuses that validated context to make one reviewed acknowledgement attempt, emits `course_run_live_progress_reported`, and then stops fail-closed

The first two bullets above are still read-only planning work. The remaining bullets define the approved Phase 4 runtime edge: one reviewed acknowledgement attempt may be replayed under fixture coverage, but the public executor still stops immediately afterward instead of widening into a broader live runtime flow.

## What `cpass run` Is Allowed To Do

The public CLI currently exposes exactly three live-task runtime signals:

1. `execution_preflight` may report a live queue entry as `registered`.
2. the event stream may emit `course_run_live_progress_reported` after the reviewed `saveTimePc` acknowledgement returns.
3. `execution_result` still marks that same entry as `blocked` after the executor stops.

When this happens, the CLI emits a warning and stops the queue after that one reviewed acknowledgement attempt. The current fixture-backed golden output proves that behavior by showing:

- a `registered` live preflight resolution
- a `running` queue transition
- a `course_run_live_progress_reported` event
- a warning that the reviewed acknowledgement route was used and the executor still stopped fail-closed
- a final `blocked` queue state

This is the approved boundary for now. Registration means the queue shape is understood, and the single reviewed acknowledgement shows the preserved runtime metadata is internally consistent. It still does not mean the live task is completed or that broader runtime execution is approved.

## Explicitly Blocked Behavior

Until a later checklist item explicitly approves live runtime contracts, do not add public runtime support for any of the following:

- live playback/session start flows
- repeated live progress or heartbeat reporting beyond the one reviewed acknowledgement attempt
- live attendance or completion acknowledgements
- public executor wiring that calls live-specific runtime endpoints other than the reviewed `saveTimePc` acknowledgement
- fixture-transport mappings for unreviewed live runtime routes beyond the reviewed `saveTimePc` acknowledgement
- executor behavior that marks a live task completed

If a future change needs any live endpoint beyond the cached planning snapshots and the reviewed `saveTimePc` acknowledgement above, it must stay out of scope until that additional runtime contract is fixture-backed, parser-covered, documented, and reviewed.

## Evidence In The Repository

The current boundary is grounded by the existing fixtures, tests, and goldens:

- fixtures:
  - `fixtures/legacy_live/chapter_cards_13.json`
  - `fixtures/legacy_live/chapter_card_attachment_13_0.html`
  - `fixtures/legacy_live/live_progress_report_live_001.txt`
  - `fixtures/legacy_live/live_progress_report_live_001_error.txt`
- core tests:
  - `builds_live_execution_context_from_preserved_queue_metadata`
  - `rejects_live_execution_context_without_iframe_data`
  - `rejects_live_execution_context_when_attachment_metadata_mismatches_iframe`
  - `builds_live_progress_report_request_from_execution_context`
  - `builds_live_progress_report_transport_request`
  - `parses_live_progress_report_ack_fixtures`
  - `rejects_live_progress_report_error_fixture`
  - `reports_live_progress_with_fixture_transport`
  - `headless_live_executor_acknowledges_fixture_backed_live_entries_and_blocks`
  - `headless_live_executor_blocks_on_rejected_fixture_acknowledgements`
  - `driver_blocks_after_acknowledging_live_progress_with_live_executor`
  - `builds_a_fixture_backed_live_course_run_plan`
  - `resolves_fixture_backed_live_queue_entries_as_registered`
- CLI coverage:
  - `plans_live_course_run_with_fixture_transport`
  - `fixtures/golden/run_course_plan_live_1001.json`

Those checks demonstrate the exact current contract: plan and classify live entries deterministically, keep the first legacy acknowledgement route fixture-backed, surface one reviewed acknowledgement on the CLI, then still stop fail-closed before attendance, completion, or broader live runtime side effects begin.

## Approval Gate For Future Live Runtime Work

Future live-task work must not skip ahead from "single reviewed acknowledgement and blocked" to "completed live runtime support." Before any broader public live executor behavior is allowed, the repo must still add:

1. separate fixture-backed transport + parser coverage for every additional live route beyond `saveTimePc`
2. explicit completion criteria proving when a live task may transition from acknowledged to completed
3. approval documentation for any repeated heartbeat cadence, attendance semantics, or completion semantics
4. CLI/test coverage that demonstrates those new runtime boundaries without regressing the current fail-closed stop point

Until then, the only supported live-task execution result on the public CLI remains `blocked`.
