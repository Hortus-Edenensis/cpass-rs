# Read-Only Boundary

This document defines the Phase 2 allowlist for Chaoxing endpoints that `cpass-rs` may call while claiming read-only behavior.

If an endpoint is not explicitly listed here as allowed, treat it as out of scope until it has been reviewed, documented, fixture-backed, and tested.

## Decision Rules

- Safety is determined by endpoint semantics, not by HTTP method alone.
- A `POST` can still be acceptable if it only returns an existing status snapshot and does not create or advance state.
- A `GET` is not automatically safe. Some exam and task routes can start attempts, advance timers, or report progress even when they are implemented as reads.
- Phase 2 commands must fail closed when the platform only exposes the required data after a start, submit, reporting, or other side-effecting step.
- The fixture transport in [`crates/cpass-core/src/transport.rs`](../crates/cpass-core/src/transport.rs) is the executable allowlist for the currently supported offline read-only surface.

## Allowed Read-Only Endpoints

These routes are currently approved for the Rust read-only commands:

| Endpoint | Used by | Why it is currently treated as safe |
| --- | --- | --- |
| `GET /apis/login/userLogin4Uname.do` | session-backed read-only commands | Refreshes account profile metadata from an existing session. It is observational and falls back to stored session data on failure. |
| `GET /mycourse/backclazzdata` | `courses list`, `courses show` | Returns the course catalog snapshot only. |
| `GET /gas/clazz` | `tasks scan` | Returns the chapter tree and chapter metadata only. |
| `POST /job/myjobsnodesmap` | `tasks scan` | Legacy API shape for chapter completion summary. The request only names existing nodes and the response is treated as progress snapshot data. |
| `GET /gas/knowledge` | `tasks scan` | Returns chapter card JSON used for task-point summaries. |
| `GET /knowledge/cards` | `tasks scan` attachment metadata | Returns the rendered chapter-card snapshot. `cpass-rs` only parses embedded attachment descriptors from it, including raw future-facing live fields such as `streamName` / `vdoid` / `liveId`, and does not invoke any follow-up reporting route. |
| `GET /ananas/status/{objectId}` | `tasks scan` video attachment metadata | Returns video duration and stream metadata for an already-known object id. |
| `GET /exam/phone/task-list` | `exam export`, `exam show`, `exam preview export` | Returns the exam catalog. The parser may extract identifiers and `enc_task` from embedded start links, but the CLI must not follow those links. |
| `GET /exam-ans/exam/phone/task-exam` | `exam export`, `exam show`, `exam preview export` | Reads the existing exam cover page and observes blocked/completed redirects without starting a new attempt. |
| `GET /exam-ans/exam/phone/preview` | `exam preview export` | Reads a previously-entered preview snapshot only when the cover metadata already exposes `exam_answer_id`, `enc`, and related preview parameters. The command fails closed if those values are missing. |

## Explicitly Unsafe Or Not Yet Approved

The following routes are outside the Phase 2 read-only boundary. Do not implement them as part of read-only parity without a new review:

| Endpoint | Status | Why it is blocked |
| --- | --- | --- |
| `GET /exam/phone/start` and legacy-observed `GET /exam-ans/exam/phone/start` | forbidden in read-only commands | These are exam entry/start routes. They may create or enter an active attempt and can start timer-sensitive flows. |
| `GET /exam-ans/exam/test/reVersionTestStartNew` | forbidden in read-only commands | Loads the active per-question exam page after an attempt has started. Not approved as a passive snapshot source. |
| `POST /exam-ans/exam/test/reVersionSubmitTestNew` | forbidden | Submits exam answers. |
| `GET /exam-ans/exam/phone/loadAnswerStatic` | forbidden | Reads answer-sheet state inside an active exam attempt and belongs to the execution flow, not the read-only export path. |
| face-detection, captcha, and other exam entry helpers used after cover-page gating | forbidden | They are part of the attempt-entry path and may unlock or progress an exam session. |
| `GET /multimedia/log/a/...` | forbidden | Video play-report route that can report playback progress or complete a task point. |
| `GET /ananas/job/document` | forbidden | Document reading-report route that can report reading state or complete a task point. |
| `GET /android/mworkspecial` | not yet reviewed | Work-task execution page; not part of the approved Phase 2 read-only surface. |
| `POST /work/addStudentWorkNew` | forbidden | Submits work-task answers. |

## Boundary Notes By Feature

### Courses

- `courses list` and `courses show` stay within catalog-only reads.
- No course command is allowed to follow links that trigger attendance, sign-in, or other course-side actions.

### Tasks

- `tasks scan` may read chapter trees, chapter status summaries, chapter card JSON, chapter-card attachment snapshots, and video status metadata.
- `tasks scan` may not call playback reporting, reading reporting, sign-in, or any other task-completion endpoint.
- Attachment exports are intentionally limited to metadata already visible from passive snapshots.
- When a chapter card exposes a live task, `tasks scan` and planning snapshots may preserve the raw live attachment descriptors (`streamName`, `vdoid`, `liveId`) from `knowledge/cards`, but runtime support still remains out of scope and fail-closed.

### Exams

- `exam export` and `exam show` are limited to catalog plus cover metadata.
- `exam preview export` is allowed only as a replay of an already-materialized preview snapshot.
- The preview path may reuse identifiers embedded in the read-only cover page, but it may not acquire them by starting an exam.
- Observing a redirect or blocked cover page is allowed; recovering from it by calling a start or submit route is not.

## Change Control For New Endpoints

Before adding any new endpoint to a read-only command:

1. Prove that the route cannot create an attempt, advance a timer, report progress, submit answers, or otherwise mutate server-side state.
2. Add or update legacy-derived fixtures and fixture-transport mappings for the new route.
3. Add parser unit tests plus offline CLI integration and golden coverage for the affected command.
4. Update this document and the Phase checklist entry in [`AGENTS.md`](../AGENTS.md) before treating the route as approved.

When there is ambiguity, keep the endpoint out of scope and fail closed.
