# Mutating exam parity contract audit

This document records the **legacy mutating exam flow** that the Python client already
implemented, plus the concrete fixture/runtime prerequisites required before the Rust
launcher can restore the same behavior without guessing about side-effecting endpoints.

It exists because the repository's current approved exam coverage is still read-only:

- exam cover metadata (`task-exam`)
- exam list
- exam preview export

The repo does **not** yet contain a reviewed mutating runtime corpus for starting,
answering, saving, or submitting an exam session.

## Legacy execution contract

Legacy source of truth:

- [`/Users/haojiejack/github/cpass-rs/main.py`](/Users/haojiejack/github/cpass-rs/main.py)
- [`/Users/haojiejack/github/cpass-rs/dialog.py`](/Users/haojiejack/github/cpass-rs/dialog.py)
- [`/Users/haojiejack/github/cpass-rs/cxapi/exam.py`](/Users/haojiejack/github/cpass-rs/cxapi/exam.py)

The legacy top-level exam path is:

1. launcher resolves `EXAM|...`
2. launcher selects one exam from the course exam list
3. `ExamDto.get_meta()` loads entry metadata from exam cover
4. legacy worker **immediately starts the exam** via `ExamDto.start()`
5. worker either:
   - exports after the exam has already started, or
   - enters question answering flow
6. worker repeatedly fetches/saves/submits answers
7. worker optionally final-submits the paper

## Legacy endpoint sequence

### 1) Read exam cover / metadata

Endpoint:

- `GET https://mooc1-api.chaoxing.com/exam-ans/exam/phone/task-exam`

Legacy params include:

- `taskrefId`
- `courseId`
- `classId`
- `userId`
- `enc_task`
- `cpi`
- `redo=1`
- `examsignal=1`

Observed legacy outputs from this step:

- `testUserRelationId` → mutable session id / `exam_answer_id`
- `monitorEnc`
- title
- gate flags:
  - `needcode`
  - `faceRecognitionCompare`
  - `captchaCheck`
  - `captchaCaptchaId`
- blocked entry states such as:
  - not started
  - chapter tasks incomplete
  - IP blocked
  - PC client only
- redirect to completed-result page if already finished

### 2) Resolve entrance challenges before start

Legacy code may perform side effects before start when cover metadata requires them:

- face-detection upload / compare flow
- captcha solve flow

These are not optional details: the `start` request conditionally carries their outputs.

### 3) Start exam session

Endpoint:

- `GET https://mooc1-api.chaoxing.com/exam-ans/exam/phone/start`

Legacy params include:

- `courseId`
- `classId`
- `examId`
- `examAnswerId`
- `cpi`
- `imei`
- `faceDetection`
- `facekey`
- `faceDetectionResult`
- `captchavalidate`
- `code`

Legacy start semantics:

- `200` may still mean start failure with HTML error body
- `302` means start success and yields redirect query state
- redirect query carries a mutable `enc`

This step is the first point where the exam is clearly running / timing.

### 4) Fetch first / next question page

Endpoint:

- `GET https://mooc1-api.chaoxing.com/exam-ans/exam/test/reVersionTestStartNew`

Legacy mutable session fields maintained across calls:

- `enc`
- `encRemainTime`
- `remainTime`
- `encLastUpdateTime`
- question form data / qid
- watermark / student display metadata

### 5) Refresh answer sheet state

Endpoint:

- `GET https://mooc1-api.chaoxing.com/exam-ans/exam/phone/loadAnswerStatic`

Legacy purpose:

- read which questions are already answered during a live session
- depends on mutable state from prior start/question fetches

### 6) Refresh whole-paper preview during live session

Endpoint:

- `GET https://mooc1-api.chaoxing.com/exam-ans/exam/phone/preview`

Important distinction:

- the repo already supports **read-only preview export** from approved preview fixtures
- legacy mutating flow uses preview again **after the exam is already started** to refresh
  live mutable state (`enc`, remain-time values, question state)

That is a different contract from the current read-only preview export feature.

### 7) Save / submit answers and final paper

Endpoint:

- `POST https://mooc1.chaoxing.com/exam-ans/exam/test/reVersionSubmitTestNew`

Legacy uses the same endpoint for:

- per-question temporary save (`tempSave=true`)
- final submit (`tempSave=false`)

Legacy request depends on dynamic state such as:

- `testPaperId`
- `testUserRelationId`
- `qid`
- `enc`
- `encRemainTime`
- `encLastUpdateTime`
- `remainTime`
- question form payload
- signature fields derived from `uid`, `qid`, cursor coordinates, etc.

Legacy response handling updates mutable session state after non-final saves and interprets
submission failures such as timeout / too-early-submit / generic submit rejection.

## Why this cannot be restored safely from current repo state

Current in-repo exam fixtures cover only read-only surfaces such as:

- [`/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_list.html`](/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_list.html)
- [`/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_cover_555001.html`](/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_cover_555001.html)
- [`/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_preview_555001.html`](/Users/haojiejack/github/cpass-rs/fixtures/legacy/exam_preview_555001.html)

What is still missing:

- reviewed `start` success/failure samples
- reviewed live-question fetch samples after start
- reviewed answer-sheet samples from a running exam
- reviewed per-question save samples
- reviewed final-submit samples
- reviewed challenge-resolution artifacts for face/captcha branches

Without those artifacts, re-implementing mutating exam behavior in Rust would require guessing:

- request parameters
- timing semantics
- redirect interpretation
- dynamic state transitions
- error branching
- whether a request mutates timers / answers / server session state

That is exactly the kind of side-effecting endpoint work the repo contract forbids unless the
runtime contract is explicitly fixture-backed and reviewed first.

## Required fixture capture contract

A minimal approved mutating exam bundle must include **one coherent session** from the same exam:

1. `exam_cover_<exam_id>.html`
2. `exam_start_success_<exam_id>.http` or equivalent request/response record
3. `exam_start_failure_<exam_id>.html` if a known gate exists
4. `exam_question_<exam_id>_0.html`
5. `exam_answer_sheet_<exam_id>.html`
6. `exam_preview_live_<exam_id>.html`
7. `exam_submit_save_<exam_id>_q<qid>.json`
8. `exam_submit_final_<exam_id>.json`

For each captured request, preserve:

- method
- full URL / query
- form body
- response status
- redirect target if any
- headers relevant to app/mobile contract
- the exact mutable fields that must be carried forward

If face or captcha is required, the bundle also needs the reviewed challenge artifacts and the
exact payload fields fed into `start`.

## Rust implementation gate after fixtures exist

Only after the reviewed bundle exists should Rust work begin in this order:

1. add parser/runtime models for mutable exam session state
2. add fixture transport mappings for start/question/answer-sheet/submit
3. add offline CLI integration tests for mutating exam flow
4. wire the interactive launcher exam worker semantics
5. document what remains unsupported (for example face/captcha branches, if still missing)

Until then, the repo should continue treating mutating exam parity as **blocked by missing
reviewed runtime contracts**, not as an implementation task that can be guessed into existence.
