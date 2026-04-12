# legacy_exam_mutating

This directory is the **tracked handoff target** for one reviewed same-session exam bundle that
restores the legacy mutating exam worker contract in Rust.

Do **not** invent placeholder HTML/JSON payloads here. Only add artifacts captured from one real,
reviewed exam session after they are sanitized and approved.

## Required core artifacts for one coherent session

All files below must come from the **same exam session** for the **same `exam_id` / `exam_answer_id`**.
Do not mix requests from different attempts.

Canonical filenames:

- `exam_cover_<exam_id>.html`
- `exam_start_success_<exam_id>.http`
- `exam_start_failure_<exam_id>.html`
- `exam_question_<exam_id>_q0.html`
- `exam_answer_sheet_<exam_id>.html`
- `exam_preview_live_<exam_id>.html`
- `exam_submit_save_<exam_id>_q<qid>.http`
- `exam_submit_final_<exam_id>.http`

## Minimum expectations per artifact

### `exam_cover_<exam_id>.html`

Must preserve the entry metadata that seeds the mutable session, including when present:

- `testUserRelationId`
- `monitorEnc`
- title
- `needcode`
- `faceRecognitionCompare`
- `captchaCheck`
- `captchaCaptchaId`
- blocked-entry messages / redirect cues

### `exam_start_success_<exam_id>.http`

Must include the full successful start request/response pair, including:

- request method
- full URL + query
- request headers that matter to the mobile-app contract
- response status
- redirect target / `Location` header
- the `enc` query value returned on success

### `exam_start_failure_<exam_id>.html`

Needed when a reviewed start failure branch exists for the same family of exam route.
Examples: bad exam code, face mismatch, other entrance rejection HTML.

### `exam_question_<exam_id>_q0.html`

Must preserve the first question page after a real successful `start`, including mutable fields such as:

- `enc`
- `encRemainTime`
- `remainTime`
- `encLastUpdateTime`
- question form payload / qid / current answer state

### `exam_answer_sheet_<exam_id>.html`

Must come from the running session and preserve the answer-sheet state that marks answered vs unanswered questions.

### `exam_preview_live_<exam_id>.html`

Must be the live preview/refresh page after the session has already started, not the existing read-only preview-export fixture.

### `exam_submit_save_<exam_id>_q<qid>.http`

Must capture one per-question save / temporary submit request and response, including:

- request query
- request form body
- response JSON
- returned mutable session fields if the server rotates them

### `exam_submit_final_<exam_id>.http`

Must capture one final-submit request/response pair from the same reviewed session.

## Current status

As of 2026-04-12 this directory is intentionally empty except for this README, because the repository
still lacks a reviewed mutating runtime exam corpus.
