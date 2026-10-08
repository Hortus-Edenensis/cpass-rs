# TaskExecutorRegistry

`TaskExecutorRegistry` is the first executor-layer bootstrap in `cpass-rs`.

It currently does two closely related things:

- take a flattened `CourseRunPlan.execution_queue` entry and resolve whether that planned task point maps to a known executor key
- walk an entire `CourseRunPlan` in stable queue order and attach that per-entry registry resolution without changing planner output
- build a fail-closed `execution_preflight` summary that `cpass run --json` can expose before the selected runtime executor stack starts dispatching queue entries

This layer is intentionally pure and side-effect free. It does not call Chaoxing, it does not emit runtime `RunEvent`s, and it does not start executing any task point yet.

Implementation entrypoints:

- `crates/cpass-core/src/task_executor.rs`: registry keys, registrations, and fail-closed queue-entry resolution
- `crates/cpass-core/src/execution.rs`: `CourseRunPlan.execution_queue`, `iter_resolved_execution_queue`, and `build_execution_preflight(...)`
- `crates/cpass-cli/src/run_output.rs`: CLI-side run payload adapter that surfaces `execution_preflight`

## Current Scope

The registry currently supports only the module families already normalized by `CourseRunner`:

| Executor key | Planned `task_kind` | Raw module |
| --- | --- | --- |
| `video` | `video` | `insertvideo` |
| `document` | `document` | `insertdoc` |
| `live` | `live` | `insertlive` |
| `chapter_work` | `chapter_work` | `work` |

Resolution is strict:

- if both `module` and `task_kind` line up with a known registration, the entry resolves to `registered`
- if the module is unknown, the entry resolves to `unsupported_module`
- if the module is known but the planned `task_kind` disagrees, the entry resolves to `inconsistent_task_kind`

That fail-closed behavior is deliberate. The registry should not silently reinterpret corrupted or hand-built queue entries, because later executor layers will depend on the planner contract remaining trustworthy.

`CourseRunPlan::iter_resolved_execution_queue(&registry)` preserves the existing flattened queue order exactly. It does not resort, filter, or mutate entries; it only pairs each queue entry with the registry's fail-closed resolution. That keeps fixture-backed tests around executor selection stable even now that module-specific runtime executors exist elsewhere in the stack.

`CourseRunPlan::build_execution_preflight(&registry)` turns that same stable queue walk into a serializable summary with per-queue resolution plus aggregate counts. The CLI now exposes it as `execution_preflight`, keyed by the same zero-based `queue_index` used in `execution_queue`.

## Safety Boundary

`TaskExecutorRegistry` consumes planner output only. It does not widen the existing read-only boundary, and it does not introduce any new endpoint.

Today it must not:

- fetch fresh task metadata
- report playback or reading progress
- open chapter work pages
- submit answers
- mutate timers, attempts, or completion state

Those behaviors belong to later Phase 3 items such as the task-specific executors themselves. The registry stays pure even though the current headless driver and CLI wiring can now hand `video`, `document`, `live`, and `chapter_work` entries to module-specific executors; `live` still stops fail-closed on the public CLI path before any live runtime endpoint is called, and `chapter_work` still stops fail-closed only after its dedicated executor attempts typed runtime snapshot discovery.

For `live`, that separation is especially important: `registered` means the queue entry shape is known to the registry, not that runtime behavior is approved. The current public boundary for live queue entries is documented in [`docs/live-task-boundary.md`](./live-task-boundary.md).

## Current Limitations

This first bootstrap still does not:

- execute queue entries
- decide retry, skip, or recovery policy

That work remains intentionally separate so the registry can stabilize its input contract before runtime behavior is added.
