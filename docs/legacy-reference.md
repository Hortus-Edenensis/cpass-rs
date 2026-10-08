# Python maintenance and Rust protocol comparison

The Python client (`main.py`, `cxapi/`, `resolver/`) is a supported maintenance runtime.
Its date-based packages ship strict four-kind question parsing and DeepSeek-compatible search.
Install the locked environment with Poetry 1.8 and Python 3.10/3.11, run
`poetry run python main.py --self-check`, then `poetry run python main.py`.

The native Rust runtime remains under `crates/`. Keep its reviewed-fixture endpoint boundaries;
a successful Python mock does not authorize a new Rust mutation route.

Maintain parsers and answer normalization at their shared boundaries. Preserve valid existing
answers, including `False`, and leave incomplete or conflicting candidates unresolved. Keep
local work caching, remote answer saves, per-question submissions, and final submission receipts
separate. Regression tests use offline HTML and mocked transports; these do not prove live
platform completion or grading correctness.

Use real sanitized captures for new article or richer-question layouts:

- `docs/article-fixture-capture.md`
- `docs/rich-question-fixture-capture.md`
- `docs/exam-mutating-parity.md` for future Rust exam mutation work

The Python packaging workflow uses locked runtime dependencies, bundled OCR resources, and
native Windows/macOS startup checks. macOS launches the console client in Terminal with a
writable user data directory. See `docs/python-searchers.md` for DeepSeek configuration.
