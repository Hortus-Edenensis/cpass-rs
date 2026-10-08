# Searcher Pipeline

`cpass-rs` is now starting the Phase 3 answer-resolution path with a shared query and
candidate contract in `crates/cpass-core/src/searcher.rs`.

## Current Scope

This Phase 3 slice still does not execute chapter-work answers or submit any runtime payload, but
it now includes the first concrete provider backends that `cpass run` can load from
`searchers[*]`: local JSON, local SQLite, HTTP answer APIs through `http` plus the legacy
`restApiSearcher` / `JsonApiSearcher` config shapes, and chat-completions-style answer APIs
through `openai-compatible` plus the legacy `OpenAISearcher` alias. The shared data shape that
later searchers and executors will share is:

- `AnswerQuerySource`: where the question came from today (`chapter_work` or `exam_preview`)
- `AnswerQuestionKind`: the normalized question-kind enum derived from the platform question type
- `AnswerQuery`: one provider-facing query carrying the prompt plus option and blank context
- `ChapterWorkQueryBatch`: a runtime chapter-work snapshot normalized into search-ready queries
- `AnswerCandidate`: one provider-attributed answer candidate
- `AnswerCandidateSelection`: one executor-facing question plus all collected candidates and the
  validated selected candidate
- `ChapterWorkCandidateSelectionBatch`: one chapter-work batch plus provider-ordered selection
  results for each question
- `SearcherPipeline`: a provider-ordered fan-out helper that preserves configured provider order
- `JsonSearcherProvider`: a local object-shaped JSON provider
- `SqliteSearcherProvider`: a local SQLite exact-match provider compatible with the legacy
  `SqliteSearcher` config shape

## Query Contract

`AnswerQuery` intentionally keeps more structure than the old Python-era `question: str`
searcher interface:

- `question_index`, `question_id`, `question_type`, and `question_type_label` preserve the
  original question identity for tracing and future executor logs
- `question_kind` normalizes known choice / blank / true-false kinds without dropping the raw
  `question_type`; it now also recognizes the broader legacy ids already named by the parser
  defaults, such as `short_answer`, `term_explanation`, `essay`, `calculation`, `matching`,
  `ordering`, `cloze`, `reading_comprehension`, `spoken`, `listening`, and `shared_option`
- `options` and `blanks` preserve the rendered question context so later JSON / SQLite / HTTP /
  OpenAI-compatible providers can decide whether to search by raw prompt only or by the full
  structured snapshot

`AnswerQuery::render_search_text()` produces a stable line-oriented representation that later
simple backends can reuse as a fallback prompt shape:

```text
[单选题 / single_choice] 普通话以哪种方言为基础方言？
A. 吴方言
B. 北方方言
```

When an option already carries preserved rich metadata, the same rendering now keeps the flattened
visible text but also appends the preserved raw option fragment metadata as advisory text instead
of dropping it:

```text
[单选题 / single_choice] 请选择图文选项
A. 图文 选项 | rich_html=<span>图文 <strong>选项</strong><img src="https://static.example/a.png" /></span> | image_urls=https://static.example/a.png
B. [no visible option text] | rich_html=<img src="https://static.example/b.png" /> | image_urls=https://static.example/b.png
```

That same option rendering now feeds the HTTP `o_field`, legacy JSON `options` object values, and
the OpenAI-compatible `{options}` placeholder whenever the parser has already preserved
`ExamQuestionOption.rich_content`. The pipeline still does not infer image meaning, perform OCR,
or claim that the raw HTML fragment is semantically complete; it only preserves the captured
metadata so downstream searchers can distinguish plain-text options from richer ones.

That normalization step is intentionally broader than the currently fixture-covered DOM layouts.
Today the repository still only has representative preview/work fixtures for the classic
single-choice, multiple-choice, fill-blank, and true-false shapes. New parser fixtures for richer
question layouts remain a separate checklist item, but the shared query contract no longer needs to
collapse those legacy ids into `unknown` when they do appear.

## Unsupported Rich Inputs

The shared query contract is still text-first. `AnswerQuery` carries:

- flattened prompt text
- flattened option text when the parser can see fixed choices
- flattened blank labels when the parser can see fill targets

The underlying parser snapshots now also preserve best-effort option-side raw fragments under
`ExamQuestionOption.rich_content.{source_html,image_urls}` when preview or chapter-work choices
already contain nested markup or `<img>` descendants. `AnswerQuery` now surfaces that preserved
option metadata in searcher-facing text, but it still does not preserve or interpret the richer
structures that often accompany the newly named legacy kinds, including:

- prompt-side HTML fragments whose meaning depends on markup rather than visible text alone
- image-only options, screenshot prompts, OCR-only stems, or other bitmap-dependent content beyond
  the preserved raw option fragment metadata
- audio/video media URLs or transcript timing for spoken/listening prompts
- shared reading/material passages with nested sub-question boundaries
- matching/ordering layouts where the answer state lives in pairings, buckets, or positional UI
- client-rendered widgets such as formula editors, canvases, or drag/drop state

When those layouts appear in future fixtures, the current pipeline may still classify the
`question_kind`, but it does not claim that `render_search_text()` is a lossless representation of
the real question. Provider outputs for such questions should therefore be treated as advisory
until fixture-backed parser fields exist for the missing structure.

The currently supported local providers are:

- JSON: accepts a local object-shaped file where each key is either the raw prompt or the rendered
  search text above, and each value is one answer string or an ordered answer array. Arrays
  stay intact for complete multiple-choice or fill-blank validation; they are never split into
  unrelated candidates. SQLite rows remain independent candidates: store a complete blank
  array as JSON text in one row, or use a complete `#`-separated answer.
  Relative `file_path` values resolve from the config file directory.
- SQLite: performs an exact-match lookup against one local table, defaulting to
  `question(question, answer)` and supporting optional `table`, `req_field`, and `rsp_field`
  overrides for legacy compatibility. It checks the raw prompt first and then falls back to the
  rendered search text when needed.

## HTTP Request Backends

The next backend in the checklist is HTTP. `HttpSearcherRequestTemplate` now normalizes three
config shapes into one Rust-side contract, and the new fail-closed `HttpSearcherProvider` reuses
that contract for core-side config validation, request dispatch, and JSON answer extraction:

- `http`: the new generic config shape, using explicit `payload_mode: form|json`
- `restApiSearcher`: legacy form/query payloads with `q_field`, optional `o_field`, `headers`,
  `ext_params`, and `a_field`
- `JsonApiSearcher`: legacy JSON-body payloads that always include `question`, `type`, and `id`

The template can deterministically turn one `AnswerQuery` into one `HttpSearcherRequest`:

- `GET + form` becomes a query-string request
- `POST + form` becomes a form-body request
- `POST + json` becomes a JSON body that preserves the legacy `question` / `type` / `id` fields
- `GET + json` is rejected during config normalization because the later provider should stay
  fail-closed rather than invent unsupported semantics

`HttpSearcherProvider` currently keeps a narrow, fail-closed boundary:

- it validates that `url` is an absolute `http` or `https` URL
- it validates custom header names and values up front instead of deferring malformed config to
  runtime
- it validates `a_field` against a deliberately small JSON-path subset: bare keys, `$.dot.paths`,
  and numeric array indexes like `[0]`
- it issues the normalized request through `reqwest`, parses the response as JSON, and then walks
  the validated `answer_path`
- it preserves answer arrays for multiple-choice and fill-blank queries and collects scalar
  answer leaves for other kinds;
  missing paths, `null`, and object-shaped payloads resolve to no candidates instead of guesses

`build_searcher_pipeline` now wires `http`, `restApiSearcher`, and `JsonApiSearcher` directly into
the public `cpass run` path, so fixture-backed course execution can pair the existing offline
transport with a local or remote answer API while keeping the executor fail-closed.

For offline replay and integration testing, the same HTTP config shapes can also set an optional
`fixture_response_path`. When present, `cpass-core` loads that JSON document from disk instead of
issuing the outbound HTTP request. Relative fixture paths resolve from the config file directory,
so the offline `cpass run` tests can keep HTTP answer replay local without changing the request
template contract.

## Chapter-Work Runtime Handoff

`ChapterWorkFormSnapshot` is still the typed runtime fetch surface produced by the current
fail-closed executor. `ChapterWorkQueryBatch` is the next read-only normalization layer on top of
that snapshot:

- it preserves the runtime work identity needed for tracing (`title`, `work_answer_id`,
  `work_relation_id`, `total_question_num`)
- it derives one `AnswerQuery` per typed runtime question in snapshot order
- it intentionally does not carry `enc_work` or any answer/save payload field so the current
  boundary stays search-only rather than submission-capable

That keeps the pipeline incremental: runtime discovery can now feed structured search queries
without widening the executor into answer mutation.

## Executor-Facing Selection Scaffolding

The current chapter-work executor can now accept an injected `SearcherPipeline` and prepare
provider-ordered candidate selections from the fetched `ChapterWorkFormSnapshot`:

- it first normalizes the runtime snapshot into `ChapterWorkQueryBatch`
- it fans each query out through `SearcherPipeline` in configured provider order
- it keeps every raw `AnswerCandidate` for review, normalizes the four classic question types,
  and selects a canonical answer only when complete, unambiguous valid candidates agree
- conflicting valid sources, partial answers, ambiguous option text, inline analysis, malformed
  JSON/fences, and unsupported kinds remain unresolved; invalid sources never win by ordering
- complete leading `<think>...</think>` blocks are excluded from the answer; embedded or
  unclosed thinking tags remain unresolved. JSON `answer` / `answers` wrappers, complete
  fences, exact option labels/text, boolean `false`, and exact blank counts are supported
- it emits only a summary-level runtime event and then still stops fail-closed before any answer
  save or submit endpoint is called

This is still intentionally just scaffolding. The preferred candidate is not yet written back into
the work form, but the public CLI can now instantiate configured `json` / legacy
`jsonFileSearcher` and `sqlite` / legacy `SqliteSearcher` providers and feed them into the
fail-closed chapter-work executor. That keeps the boundary explicit: search-only normalization and
strict selection are now possible inside the executor contract, but mutation remains blocked until later
Phase 3 items land.

That fail-closed boundary is especially important for richer or media-heavy question layouts. The
executor must not compensate for missing parser structure by fabricating answer payloads, scraping
additional runtime pages, or widening into save/submit routes. Until representative fixtures prove
how those layouts are encoded, the only supported behavior is to preserve the normalized question
kind, collect zero or more advisory candidates, emit summary events, and stop before any answer
mutation endpoint is called.

## OpenAI-Compatible Backend

`build_searcher_pipeline` now also wires `openai-compatible` plus the legacy
`OpenAISearcher` alias into the public `cpass run` searcher pipeline. The Rust core keeps the
provider deliberately narrow and fail-closed:

- `OpenAiCompatibleRequestTemplate` validates `base_url`, `model`, and `api_key`, normalizes the
  configured API root onto `/chat/completions`, and redacts the API key in debug output
- `system_prompt` stays optional; when absent the contract falls back to a narrow answer-only
  instruction
- prompt rendering accepts either legacy `prompt` or newer `prompt_template` config, defaults to
  `{search_text}`, and supports placeholder substitution for `{type}`, `{value}`, `{question}`,
  `{options}`, `{blanks}`, and `{search_text}`
- `OpenAiCompatibleRequest` narrows the outbound payload to one system message plus one user
  message derived from the normalized `AnswerQuery`, with optional validated `thinking`,
  `reasoning_effort`, `max_tokens`, and `response_format` fields; unset fields are omitted
- `thinking` accepts only `{type: enabled}` or `{type: disabled}`. `reasoning_effort` accepts
  `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, or `ultra`; contradictory thinking
  controls are rejected. `max_tokens` must be a positive integer
- `response_format` accepts only `{type: text}` or `{type: json_object}`. JSON mode appends an
  instruction and examples such as `{"answer":"A"}` and `{"answers":["A","C"]}` to the system
  prompt so the requested format has an explicit answer contract
- `OpenAiCompatibleResponse` accepts the common `choices[*].message.content` chat-completions
  shape. Separate `reasoning_content` is ignored and never used as an answer. Explicit
  `finish_reason` values other than `stop` produce no candidate; absent values remain compatible
  with existing gateway/fixture responses
- `OpenAiCompatibleSearcherProvider` sends one authorized chat-completions request per
  `AnswerQuery`, returns at most the first non-empty `choices[*].message.content` answer
  candidate, and supports backend injection or fixture-backed responses for isolated tests
- relative `fixture_response_path` values resolve from the config file directory, so offline
  `cpass run` integration tests can replay chat-completions responses without issuing live HTTP
  calls
- `CPASS_OPENAI_API_KEY` can still inject the `api_key` field for `openai-compatible`,
  `OpenAISearcher`, or `openai` searcher entries at config-load time, keeping secrets outside the
  checked-in YAML samples

DeepSeek-V4.1-Flash uses the official API model id `deepseek-flash`. When the configured URL host
is exactly `api.deepseek.com`, `deepseek-v4.1-flash` is accepted as a local alias for that model;
gateway model names are passed through unchanged. On the official host only, effort aliases map
`minimal` to `low`, `medium`/`xhigh` to `high`, and `ultra` to `max`, and `max_tokens` is capped at
393216. Thinking can remain enabled while the answer pipeline consumes only the final content;
this searcher uses one-shot Chat Completions without tools or conversational reasoning replay.
The current request contract follows the official [model list](https://api-docs.deepseek.com/api/list-models/),
[thinking controls](https://api-docs.deepseek.com/guides/thinking_mode/),
[Chat Completions schema](https://api-docs.deepseek.com/api/create-chat-completion/), and
[JSON output requirements](https://api-docs.deepseek.com/guides/json_mode/).

That means the public CLI can now use local JSON, SQLite, HTTP, or OpenAI-compatible answer
providers to prepare chapter-work candidate selections from the same typed runtime snapshot while
still stopping fail-closed before any answer save or submit endpoint is called.

## Handoff Boundary

The shared searcher pipeline is still intentionally narrower than the later execution work:

- no chapter-work answer fill or submission logic yet
- no answer submission or save routes
- JSON, SQLite, HTTP, and OpenAI-compatible providers are wired into `cpass run` today; all of
  them stay on the same search-only side of the executor boundary

Those behaviors remain separate checklist items so the current Rust core can add question-query
normalization first, then wire provider backends, and only after that connect candidates to the
fail-closed runtime flow.
