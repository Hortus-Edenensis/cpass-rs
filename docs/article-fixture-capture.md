# Article Task Fixture Capture Contract

This document records the current fixture-capture contract for the pending article-reading
(`insertbook`-style) task family.

The repository still does not contain a real article task-point sample, so parser, planning, and
runtime work remain blocked. The goal here is narrower: make the eventual capture format explicit so
future fixture drops line up with the existing transport conventions instead of inventing a new
layout ad hoc.

## Current Blocker Audit

As of 2026-04-12, a repository audit still shows that article-reading work is blocked by missing
source evidence rather than missing parser or executor plumbing:

- an expanded 2026-04-12 audit across tracked files, git history (`git log -S insertbook`,
  `git rev-list --all --objects`), and local build/OMX artifacts still only finds this document,
  the AGENTS checklist, and the placeholder `fixtures/legacy_article/README.md`; there is still no
  real article fixture bundle or runtime payload sample checked into the workspace
- `fixtures/legacy_article/` now exists only as a placeholder README, so there is still no real
  chapter-card JSON or `knowledge/cards` attachment HTML to lock before parser work starts
- the legacy Python reference in this repository also does not expose an `insertbook` handler or a
  reviewed article progress-report route that could be used as a safe contract substitute

That means the next article task is still an external capture handoff: obtain one real task-point
sample first, then resume the checklist in order without guessing at module names or runtime
endpoints.

## Canonical Fixture Root

When a real article task sample is available, store the captured payloads under:

- `fixtures/legacy_article/`

That keeps the article module family separate from the existing:

- `fixtures/legacy/` read-only parity fixtures
- `fixtures/legacy_run_document/` document runtime fixtures
- `fixtures/legacy_live/` live runtime fixtures

## Canonical File Names

The chapter-card and attachment captures should follow the same naming shape already used by the
fixture transport for other task families:

- `fixtures/legacy_article/chapter_cards_<knowledgeid>.json`
  - source route: `GET /gas/knowledge?id=<knowledgeid>`
  - purpose: preserve the raw task-point list so parser work can confirm the real article module
    name, resource id fields, and queue ordering.
- `fixtures/legacy_article/chapter_card_attachment_<knowledgeid>_<num>.html`
  - source route: `GET /knowledge/cards?knowledgeid=<knowledgeid>&num=<num>`
  - purpose: preserve the attachment snapshot that should expose the article-specific identifiers
    needed for later read-only metadata parsing and runtime review.

Do not invent a fake progress-report filename before a real sample exists. The article runtime
acknowledgement route is still unknown, so its eventual fixture name must be derived from the real
request path plus the real response encoding after capture. Until then:

- do not add a `FixtureChaoxingTransport` mapping for article runtime traffic
- do not add placeholder article progress fixtures with guessed JSON or text shapes
- do not infer that the runtime route mirrors `insertvideo`, `insertdoc`, or `insertlive`

## Minimum Capture Bundle

When the first real article sample is captured, the bundle should include all of the following from
the same task point:

1. one chapter-card JSON snapshot
2. one matching `knowledge/cards` attachment HTML snapshot
3. the exact article runtime progress-report request details
4. the exact runtime acknowledgement response body and content type

The captured request details should retain the raw identifiers that tie the three artifacts
together, including whichever of the following are present on the real sample:

- `knowledgeid`
- card index / `num`
- module name (for example `insertbook`, if that is what the legacy payload really uses)
- task resource id
- attachment object id / job id / token fields
- any article-specific runtime identifiers carried by the attachment snapshot or iframe payload

## Acceptance Gate

Article parser or runtime work should stay blocked until the repository has:

1. a real article chapter-card JSON snapshot
2. a real matching `knowledge/cards` attachment snapshot
3. a reviewed runtime request/response pair for the article acknowledgement route

Only after those three artifacts exist should follow-up work add:

- fixture transport mappings
- parser models and unit tests
- task-scan / planning metadata preservation
- fail-closed executor wiring
- golden outputs and offline CLI coverage

Until then, this document is the only approved contract: reserve the fixture root and chapter-card /
attachment naming shape, but do not guess the runtime route.
