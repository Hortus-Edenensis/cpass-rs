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

If the reviewed session hits optional face/captcha gates before `start`, keep those artifacts in
this directory too. Name them from the real observed branch/route after capture instead of
inventing guessed filenames up front.

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

## Optional challenge branches

### Face gate

When the cover metadata enables `faceRecognitionCompare`, also capture the reviewed face branch from
the same session, including:

- the request/response pairs the legacy flow actually used before `start`
- any reviewed failure HTML/message for face mismatch or other face-entry rejection
- the exact `faceDetection`, `facekey`, and `faceDetectionResult` values eventually sent to
  `exam_start_success_<exam_id>.http`

### Captcha gate

When the cover metadata enables `captchaCheck` or provides `captchaCaptchaId`, also capture the
reviewed captcha branch from the same session, including:

- the challenge payload/page that exposes the captcha state
- the verification request/response pair
- the exact `captchavalidate` value eventually sent to `exam_start_success_<exam_id>.http`
- the `code` value when the same reviewed branch still requires a manual exam code

If both gates appear, preserve the real branch ordering from the reviewed session.

## Sanitization rules

Sanitization is allowed only if the session contract survives intact:

- keep route paths, parameter keys, response structure, status codes, and redirect headers
- replace secrets/PII with stable placeholders across the whole bundle when they are not needed to
  model the protocol, including cookies, session tokens, device identifiers such as `imei`,
  student-identifying fields, face images/blobs, captcha answers, raw exam-code values, and IP data
- reuse the same placeholder when the same sensitive value appears again later in the session
- do **not** redact the mutable exam/session fields that tie the bundle together:
  - `examId`
  - `courseId`
  - `classId`
  - `cpi`
  - `testUserRelationId` / `examAnswerId`
  - `testPaperId`
  - `qid`
  - `enc`
  - `encRemainTime`
  - `encLastUpdateTime`
  - `remainTime`
  - gate flags and the face/captcha outputs fed into `start`

## Per-request metadata that must stay paired

Every reviewed request/response artifact in this directory should preserve:

- request step/order inside the same session
- method
- full URL / query
- form body or equivalent request payload
- response status
- response content type
- redirect target / `Location`, when present
- the session identifiers shared across the bundle:
  - `examId`
  - `courseId`
  - `classId`
  - `cpi`
  - `testUserRelationId` / `examAnswerId`
- the mutable state carried forward between artifacts:
  - `enc`
  - `encRemainTime`
  - `encLastUpdateTime`
  - `remainTime`
- question identifiers when present:
  - `qid`
  - `testPaperId`
- gate/challenge fields when present:
  - `needcode`
  - `code`
  - `faceRecognitionCompare`
  - `faceDetection`
  - `facekey`
  - `faceDetectionResult`
  - `captchaCheck`
  - `captchaCaptchaId`
  - `captchavalidate`

## Current status

As of 2026-04-12 this directory is intentionally empty except for this README, because the repository
still lacks a reviewed mutating runtime exam corpus. The expanded 2026-04-12 audit across tracked
files, git history, and local build/OMX artifacts found only the legacy Python reference, read-only
docs/entry fixtures, and this placeholder/capture-contract surface; there is no checked-in `start` /
question / answer-sheet / save / final-submit bundle to canonicalize yet.
