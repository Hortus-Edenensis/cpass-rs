# Richer Question Fixture Capture Contract

This document records the current capture contract for richer question layouts beyond the classic
single-choice, multiple-choice, fill-in-the-blank, and true-false shapes that the repository
already proves today.

The goal is intentionally narrow: make the current blocker explicit and define how future fixture
drops should be organized once a real non-classic sample exists. It does **not** approve new parser
assumptions or runtime behavior before those samples are captured.

## Current Blocker Audit

As of 2026-04-12, the repository still lacks any real preview or chapter-work DOM sample for the
non-classic legacy question-type ids already enumerated in `cxapi/schema.py`.

The current checked-in fixture corpus only proves:

- `fixtures/legacy/exam_preview_555001.html` and
  `fixtures/legacy_run_document/exam_preview_555001.html` expose preview `question_type` values
  `0`, `1`, `2`, and `3`
- `fixtures/legacy/chapter_work_11_work_001.html` and
  `fixtures/legacy_run_document/chapter_work_11_work_001.html` expose chapter-work `answertype`
  values `0`, `2`, and `3`
- `fixtures/legacy_rich_options/exam_preview_555001.html` and
  `fixtures/legacy_rich_options/chapter_work_11_work_001.html` add rich-option markup, but they
  still only prove type-`0` question wrappers

Meanwhile, the legacy Python enum still advertises richer ids such as:

- `4` short answer
- `5` term explanation
- `6` essay
- `7` calculation
- `9` journal entry
- `10` material
- `11` matching
- `13` ordering
- `14` cloze
- `15` reading comprehension
- `18` spoken
- `19` listening
- `20` shared option
- `21` assessment

Those ids are already normalized and surfaced by the Rust planning/search contracts, but the
repository does not contain any checked-in preview/work HTML that proves how those richer layouts
actually render. That means parser support is blocked by missing fixture evidence, not by missing
type-name plumbing.

## Planned Fixture Root

When the first real richer-question sample becomes available, store it under:

- `fixtures/legacy_rich_questions/`

Do not reuse `fixtures/legacy_rich_options/` for this work. That root already means "classic
question types with richer option markup", which is a narrower case than the broader non-classic
layout families listed above.

## Canonical File Names

Keep future fixture names aligned with the existing transport conventions for the source surface
that produced the sample:

- exam preview sample:
  - `fixtures/legacy_rich_questions/exam_preview_<exam_id>.html`
  - optional supporting context: `exam_cover_<exam_id>.html` when the preview depends on cover
    metadata already used by fixture-backed preview export
- chapter-work sample:
  - `fixtures/legacy_rich_questions/chapter_work_<knowledgeid>_<workid>.html`
  - optional supporting context: `chapter_cards_<knowledgeid>.json` and
    `chapter_card_attachment_<knowledgeid>_<num>.html` when the work sample depends on attachment
    discovery already used by fixture-backed task scanning / planning

If the richer layout depends on referenced media URLs, nested fragments, or shared-stem wrappers,
preserve the raw HTML snapshot that contains those references before trying to normalize them into
smaller synthetic files.

## Minimum Capture Bundle

Before parser work resumes, the first richer-question fixture bundle should include all of the
following for the same real sample:

1. one real preview or chapter-work HTML snapshot containing at least one non-classic question type
   or a materially richer layout than the current classic fixtures prove
2. the matching cover/card/attachment context needed to address that snapshot through the existing
   fixture-backed transport flow
3. the raw question-type ids visible in that snapshot, preserved as rendered rather than rewritten
   into hand-made fixture text

Helpful but optional supporting evidence:

- screenshots that explain nested/shared-stem layouts
- notes about whether the sample came from exam preview or chapter-work runtime discovery
- any media URLs or DOM subtrees whose structure matters for later parser modeling

## Acceptance Gate

Do not widen richer-question parser support until the repository has a real fixture bundle under
`fixtures/legacy_rich_questions/` that proves the target layout. In particular:

- do not infer matching/material/shared-option nesting from enum names alone
- do not guess at spoken/listening media structures without a real DOM sample
- do not claim support for ordering, matrix, drag/drop, or shared-stem layouts until those DOM
  shapes are captured and covered by unit tests

Until that bundle exists, the current approved behavior remains:

- preserve the raw `question_type` and normalized `question_kind`
- surface the richer kind metadata in plans, run output, and search queries
- fail closed on parser changes that would require unproven DOM structure
