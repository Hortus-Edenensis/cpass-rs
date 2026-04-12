# ExamRunner

`ExamRunner` is the current Phase 3 exam-planning bootstrap in `cpass-rs`.

Like `CourseRunner`, it is intentionally limited to planning. It resolves one course plus one exam, reuses the existing read-only exam catalog, cover metadata, and preview-question inventory pipeline, emits planning lifecycle `RunEvent`s, and builds a deterministic `ExamRunPlan` snapshot for later executor work.

Implementation entrypoints:

- `crates/cpass-core/src/execution.rs`: `ExamRunner`, `ExamRunTarget`, `ExamRunPlan`, and question-kind mapping
- `crates/cpass-core/src/event.rs`: `exam_run_planning_*` events plus the shared `RunEventSink` contract
- `crates/cpass-cli/src/run_output.rs`: CLI-owned event buffering and JSON output adapters reused by run-oriented commands

There is no dedicated CLI command for exam execution planning yet. Today this contract is exposed as a core-layer planning primitive for future headless execution and UI work.

## UI Boundary

`ExamRunner` follows the same boundary as `CourseRunner`. Its public contract is the returned `ExamRunPlan` plus optional `RunEvent` emission through `with_sink(Arc<dyn RunEventSink>)`.

It does not render UI state, print progress, or depend on CLI/TUI-specific types. Any future headless shell or interactive UI should subscribe to `RunEvent` externally and keep presentation logic outside `cpass-core`. The current CLI and test coverage keep event buffering on the caller side instead of defining a production collector inside `cpass-core`.

## Current Scope

The current runner does exactly six things:

1. Emit `exam_run_planning_started` with the caller's requested `course_id` / `course_index` and `exam_id` / `exam_index` selector.
2. Resolve the target course from the read-only course catalog.
3. Resolve the target exam from the read-only exam catalog for that course.
4. Fetch read-only exam cover metadata for the selected exam.
5. Derive the preview query from that metadata and load the read-only preview question inventory.
6. Convert that snapshot into a stable `ExamRunPlan`, then emit `exam_run_planning_finished` with the resolved course id, exam id, and question count.

The result is a planning artifact only. It is safe to inspect, save, diff, or hand to future executor layers, but it is not an active exam session and it is not an answer submission engine.

## Current Data Contract

`ExamRunPlan` currently includes:

- the resolved course identity and metadata
- the resolved exam catalog entry plus its read-only cover metadata
- the exact `preview` query derived from the cover metadata and used to address the already-materialized preview snapshot
- one normalized `ExamRunQuestionPlan` per preview question
- a flattened `total_questions` count

Each `ExamRunQuestionPlan` currently preserves:

- `question_index`
- `question_id`
- raw `question_type`
- server-provided `question_type_label`
- normalized `question_kind`
- `prompt`
- `options` when the preview exposes fixed choices
- `blanks` when the preview exposes fill-in placeholders

The planner sorts questions deterministically by question index, question id, question type, type label, and prompt. Unknown question types remain visible in the plan instead of being dropped or coerced into a supported kind.

## Supported Question Kinds

The planner currently has fixture-backed parser coverage for the classic preview shapes already in
this repository, but its normalized `question_kind` mapping now recognizes the broader legacy
question-type ids already enumerated by the parser defaults:

| Raw `question_type` | Planned `question_kind` | Current meaning |
| --- | --- | --- |
| `0` | `single_choice` | One correct option selected from the preview options |
| `1` | `multiple_choice` | Multiple correct options selected from the preview options |
| `2` | `fill_blank` | One or more blank placeholders preserved in `blanks` |
| `3` | `true_false` | True/false judgment question |
| `4` | `short_answer` | Free-form short-answer prompt with no fixed options |
| `5` | `term_explanation` | Definition-style prompt, preserved without option coercion |
| `6` | `essay` | Long-form discussion prompt, preserved without option coercion |
| `7` | `calculation` | Calculation-oriented prompt, preserved without option coercion |
| `8` | `other` | Legacy catch-all type preserved as a named kind |
| `9` | `journal_entry` | Legacy accounting-entry style prompt |
| `10` | `material` | Material / source-based prompt family |
| `11` | `matching` | Matching prompt family |
| `13` | `ordering` | Ordering / sequence prompt family |
| `14` | `cloze` | Cloze-style fill prompt family |
| `15` | `reading_comprehension` | Reading-comprehension prompt family |
| `18` | `spoken` | Spoken/oral prompt family |
| `19` | `listening` | Listening prompt family |
| `20` | `shared_option` | Shared-option prompt family |
| `21` | `assessment` | Assessment / evaluation prompt family |
| any other value | `unknown(<question_type>)` | Preserved for later executor and parser support instead of being dropped |

Unknown question kinds are not treated as planning errors. They remain part of the plan so future
fixture capture, parser work, and execution support can extend behavior without changing the
planning contract. Richer preview DOM fixtures for the newly named kinds are still a separate
checklist item; until they land, the planner classifies those ids when present but does not claim
broader parser-layout coverage than the repository can prove today.

## Unsupported Rich Question Structures

The current preview parser is still intentionally narrow. It only proves the DOM shapes already
covered by repository fixtures:

- one question wrapper per `div.questionWrap.singleQuesId.ans-cc-exam`
- one title block under `div.tit`
- text-first option extraction from `div.answerList.radioList`, with optional raw rich-content
  preservation when an option node contains nested markup or `<img>` descendants
- plain-text blank labels from `div.completionList.objectAuswerList span.grayTit`

That means the richer legacy kinds listed above are currently normalized by `question_type`, but
not yet structurally modeled beyond the text the existing selectors can already see. In
particular, the planner now preserves raw option fragments under
`ExamQuestionOption.rich_content.{source_html,image_urls}` when preview choices already expose
nested markup or images, but it still does not interpret:

- prompt-side rich-text fragments beyond their flattened visible text
- figure-only prompts, OCR-required answer choices, or any semantic meaning that depends on image
  understanding
- audio/video-backed prompts such as spoken or listening questions
- material passages with nested sub-questions or shared stems
- matching pairs, ordering buckets, drag/drop regions, or matrix/table answer layouts
- math editors, formula widgets, canvases, or any client-rendered interaction state

For those layouts, `ExamRunPlan` keeps the lossless identifiers it already has (`question_id`,
`question_type`, `question_type_label`) plus whatever flattened `prompt`, `options`, and `blanks`
the current selectors can extract, alongside the best-effort raw option fragment metadata noted
above. It does not claim that the rendered structure is complete.

The parser also stays fail-closed on missing core preview structure. If a preview page no longer
contains the expected question wrapper, question id input, question type input, or title block, the
planner returns an error instead of guessing at a richer layout.

## RunEvent Coverage

When a caller wires a `RunEventSink` implementation such as the CLI-side `RunEventBuffer` into `ExamRunner`, the planner emits:

- `exam_run_planning_started`: emitted before course resolution, exam resolution, and preview loading, with the caller's raw course/exam selector
- `exam_run_planning_finished`: emitted after the plan is built, with the resolved `course_id`, `exam_id`, and `questions` count

These events describe planning progress only. They do not imply exam entry, timer start, answer lookup, answer submission, or any other side effect.

## Preview-Based Safety Boundary

`ExamRunner` is only allowed to reuse the same safe read-only inputs already approved for exam inspection:

- `GET /mycourse/backclazzdata` for course catalog resolution
- `GET /exam/phone/task-list` for exam catalog resolution
- `GET /exam-ans/exam/phone/task-exam` for read-only cover metadata
- `GET /exam-ans/exam/phone/preview` for a previously-entered preview snapshot

That boundary is intentionally narrower than “anything that returns exam data.” The planner must fail closed unless the read-only cover metadata already exposes the preview parameters required to address an existing preview snapshot, especially `exam_answer_id`.

The current implementation therefore refuses to plan when the cover metadata does not expose a usable preview query, such as:

- completed exam flows that redirect away from a reusable preview snapshot
- blocked cover pages that do not reveal preview parameters
- any state where the platform would require an exam-start or exam-entry route to obtain the next identifier

The planner must not cross that boundary by calling:

- `/exam/phone/start`
- legacy-observed exam start or active-question routes
- answer-sheet or answer-submission routes
- any face-check, captcha, timer, or entry helper tied to starting or advancing an attempt

For the repository-wide allowlist and denylist, see [`docs/read-only-boundary.md`](./read-only-boundary.md).

That same fail-closed posture also applies to richer question layouts. Missing structure or missing
fixture-backed semantics must not be “fixed” by widening the endpoint boundary, loading active
question pages, or inferring new media routes from client scripts. Rich-text and media-heavy exam
support remains blocked until representative preview fixtures prove the layout and the existing
read-only boundary still holds.

## Unsupported Behavior

Until executor wiring lands, `ExamRunner` does not:

- start or resume an exam attempt
- open active per-question exam pages
- look up answers from search providers
- fill or submit answers
- save drafts, submit papers, or advance timers
- emit runtime `RunEvent`s beyond the planning start/finish markers above
- recover from blocked/completed cover pages by widening the endpoint boundary

This means `ExamRunner` is currently closer to `exam show + exam preview export + normalization` than to a runnable exam executor.

## Executor Handoff Boundary

The current handoff boundary is the serialized `ExamRunPlan` itself:

- future executor layers can consume `questions` in ascending `question_index` order without re-sorting the preview snapshot
- `question_kind` is a convenience classification, but `question_type` and `question_type_label` remain the lossless source fields
- `preview` is the exact read-only addressing context used for planning and should remain inspectable for later executor work
- `total_questions == questions.len()` is the current planning invariant for progress baselines

Anything beyond that belongs to later Phase 3 items such as `TaskExecutorRegistry`, the unified searcher pipeline, and the real headless execution path.
