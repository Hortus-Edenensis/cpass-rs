# Legacy Reference

The Python implementation in this repository is now treated as a legacy reference source for:

- protocol reverse-engineering
- fixture generation
- protocol-comparison handoffs during the Rust rewrite

It is no longer the primary delivery target. New development should land in the Rust workspace under `crates/`.

Legacy directories and files kept for reference:

- `cxapi/`
- `resolver/`
- `main.py`
- `dialog.py`
- `config.py`
- `utils.py`
- `pyproject.toml`
- `poetry.lock`

Migration rules for the rewrite:

- Prefer adding fixtures and parser tests before porting a legacy workflow.
- Keep `config.yml` compatibility where reasonable during the bootstrap phase.
- Do not add new feature work to the Python implementation unless it is required to capture fixtures or explain behavior.
- Any unavoidable Python-side diff should point to the Rust-owned fixture, regression test, or protocol note that will carry the durable handoff.
- Use `docs/article-fixture-capture.md` when collecting the first article-reading samples so the eventual fixture drop matches the existing transport naming conventions without inventing unsupported runtime contracts.

## Allowed legacy workflows

Use the Python environment only for tasks that the Rust workspace cannot already express directly:

1. capture a real request/response pair or rendered page into `fixtures/`
2. compare a Rust parser/runtime result against legacy behavior during a port
3. annotate undocumented protocol details that still need to be carried into Rust contracts

Everything else should default back to `crates/`:

- user-facing commands and CLI output
- parser models and transport contracts
- tests, golden baselines, and fixture-backed integrations
- docs that describe the supported runtime

## Legacy tooling expectations

- `poetry install` is an opt-in reference workflow, not part of the default bootstrap, CI, release, or Docker path
- if Python is used during an investigation, the final checked-in artifact should usually be a fixture, a Rust test, or a protocol note rather than a new Python feature
- when a Python diff is unavoidable, keep it reversible and explain which Rust follow-up artifact depends on it
- once the Rust-side handoff artifact exists, stop extending the Python path and move follow-up work back under `crates/`, `fixtures/`, or docs

## Repository guidance checkpoints

Use these checkpoints when auditing docs or review notes so the repository keeps framing Python as
legacy-only:

- `AGENTS.md` should continue to treat Python as legacy reference scope and keep the active
  delivery checklist focused on Rust workspace work under `crates/`
- root guidance should continue to point feature work, user-facing commands, and validation to the
  Rust workspace under `crates/`
- `pyproject.toml`, `poetry.lock`, and `main.py` should read like preserved investigation tooling,
  not like the default bootstrap or release path
- CI, release, and Docker guidance should continue to describe the Rust CLI as the shipped runtime
  instead of reviving a Poetry / Python delivery path
- when a new doc or review note must mention Python, it should normally link back to this page
  instead of restating Poetry / `main.py` as a default workflow
- command examples that mention Poetry or `python main.py` should be labeled as legacy-only helper
  flows for fixture capture or protocol comparison
- Python-only diffs should state the Rust-side handoff they unblock (fixture capture, protocol
  comparison, or protocol notes) so reviews do not accidentally treat them as new product surface

When a change touches repository guidance or only Python-side files, explicitly document the
Rust-side handoff in the same change (for example: fixture capture notes, parser fixtures, protocol
comparison evidence, or a follow-up contract in `docs/`). If that handoff is missing, the change
probably belongs in Rust-owned artifacts under `crates/`, `fixtures/`, tests, or documentation
instead of expanding the legacy Python path.

## Legacy-only command examples

These commands are intentionally framed as investigation helpers, not as a supported runtime path:

```bash
# Legacy-only: prepare the old Python lab bench for fixture capture / protocol comparison.
poetry install

# Legacy-only: run the preserved Python entrypoint only while capturing or comparing legacy behavior.
poetry run python main.py
```

For normal development, validation, releases, and user-facing automation flows, default back to the
Rust CLI under `crates/`.
