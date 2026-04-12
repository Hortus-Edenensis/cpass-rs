# Automation Profiles

This document captures the Rust-side profile recipes that are ready for automation today, plus the
remaining boundaries where callers still need to make an explicit operational choice.

## Profile Selection and Merge Rules

`AppConfig::load_from_path_with_profile` applies one named profile on top of the root config:

- select it with `--profile <name>` or `CPASS_PROFILE=<name>`
- `session_path`, `log_path`, `export_path`, `face_image_path`, `transport`, and `login` inherit
  from the root config unless the profile overrides a field
- `searchers` and `notifications` replace the root lists only when the profile sets them
- `config validate --json` and `doctor --json` surface both `selected_profile` and
  `automation_paths`, so scripts can inspect the effective directories without reimplementing path
  normalization

For session-backed commands, the CLI resolves the session selector in this order:

1. `--phone`
2. the effective `login.phone` after profile selection plus any `CPASS_PHONE` override
3. the only saved session under the effective `session_path`

If multiple saved sessions remain after that resolution, the command fails closed with a selector
error instead of guessing.

## Recommended Automation Recipes

### 1. Validate the selected profile before doing work

Use `config validate` and `doctor` as the preflight pair for scheduled or CI-style automation:

```bash
CPASS_PROFILE=automation cargo run -p cpass-cli -- config validate --json
CPASS_PROFILE=automation cargo run -p cpass-cli -- doctor --json
```

These commands are the safest place to confirm that:

- the intended profile exists
- relative `session/`, `log/`, `export/`, and `face/` paths normalize to the expected workspace
  directories
- the selected profile exposes the expected `searchers` and `notifications` count
- the effective session directory already contains the saved session that later export/run commands
  will need

### 2. Refresh a saved session for a pinned automation profile

`cpass login` is scriptable, but it is still the profile recipe that most directly depends on
external secrets and a live account:

```bash
export CPASS_PROFILE=automation
export CPASS_PHONE=13800138000
export CPASS_PASSWORD='***'
cargo run -p cpass-cli -- login --json
```

Operational notes:

- `login` does not prompt on stdin; it requires a phone and password from `--phone` / `--password`,
  the selected profile's `login` block, or `CPASS_PHONE` / `CPASS_PASSWORD`
- omit `--no-save` for the normal automation case so the refreshed session lands under the
  profile-specific `session_path`
- prefer environment variables for secrets; keep `config.example.yml` or checked-in profile stanzas
  password-free

### 3. Export deterministic exam snapshots

Once the automation profile pins the saved-session selector, the read-only export commands become
fully non-interactive as long as the caller also supplies explicit course/exam selectors:

```bash
CPASS_PROFILE=automation cargo run -p cpass-cli -- \
  exam export --course-id 1001 --output export/automation/exam_catalog_1001.json --json

CPASS_PROFILE=automation cargo run -p cpass-cli -- \
  exam preview export --course-id 1001 --exam-id 555001 \
  --output export/automation/exam_preview_1001_555001.json --json
```

Recommendations for automation callers:

- pass `--output` when the surrounding job expects a stable filename; otherwise the CLI writes to
  the profile's `export_path` using the default manifest filename
- pass `--course-id` / `--exam-id` instead of relying on index-based selection when the surrounding
  system already knows the remote identifiers
- keep `--fixture-dir` for offline replay tests only; production automation should omit it so the
  saved session and live transport remain aligned

### 4. Run the headless execution path with explicit profile defaults

The supported automation-oriented `run` recipe is the headless JSON path:

```bash
CPASS_PROFILE=automation cargo run -p cpass-cli -- run --course-id 1001 --json
```

This lets one profile carry:

- the saved-session selector via `login.phone`
- any configured `searchers` used for answer-candidate lookup
- any configured CLI-owned `notifications` used to summarize the terminal `RunEvent` stream

The JSON result stays the best automation handoff because it includes the deterministic plan, the
executor preflight summary, and the runtime execution result in one payload.

## Remaining Interactive Boundaries

### `login`

`login` is scriptable, but it is not a fire-and-forget read-only command:

- it requires a real phone/password credential pair; the CLI does not offer an interactive prompt
  fallback
- it performs a live authentication attempt and may still fail for account-side reasons that the
  automation cannot resolve by retrying blindly
- callers need to decide whether the session should be persisted (`default`) or intentionally kept
  ephemeral (`--no-save`)

### `exam export` and `exam preview export`

The export commands are otherwise non-interactive, but automation still has to provide the final
selection inputs explicitly:

- `exam export` needs one course selector: `--course-id` or `--course-index`
- `exam preview export` needs both a course selector and an exam selector
- preview export also depends on read-only cover metadata containing `exam_answer_id`; if that
  metadata is unavailable, the command fails closed instead of trying a start/submit path

### `run`

The automation-safe `run` boundary is the headless CLI path only:

- prefer `--json`; it is stable for scripting and keeps all lifecycle state in one payload
- `--tui` intentionally switches to a subscriber UI and therefore remains an operator-facing mode,
  not a background automation recipe
- unsupported or review-pending task families still stop fail-closed instead of guessing at a
  write-side endpoint
- chapter-work answer submission remains outside the supported boundary even when searchers can
  derive candidate answers

## Suggested Profile Shape

```yaml
profiles:
  automation:
    session_path: "session/automation"
    export_path: "export/automation"
    login:
      phone: "13800138000"
    transport:
      retries: 5
    searchers:
      - type: json
        file_path: "questions.json"
    notifications:
      - type: gotify
        base_url: "https://gotify.example/message"
        token: "gotify-app-token"
```

Keep passwords and supported API keys in environment variables, and keep notification tokens in the
profile config until that configuration path gains its own secret source. Then use
`config validate --json` or `doctor --json` to confirm the effective profile before scheduling
login, export, or run jobs.
