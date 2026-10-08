# MQTT Live Delivery Guardrails

This note records the guardrails that govern the repository's live MQTT delivery path.
It began as the explicit approval gate for a future live backend and now also documents the
landed implementation shape so later work does not need to reconstruct the same transport boundary
from commit history alone.

## Approved live-delivery shape

Keep one **CLI-only** live MQTT delivery path with a Rustls-first TLS story. The implementation may
reuse already-available workspace crates instead of fetching a new MQTT client dependency, but it
must preserve these guardrails:

- keep all broker state and publish flow inside `crates/cpass-cli`; `cpass-core` stays transport-agnostic
- continue supporting the existing `fixture_response_path` replay mode as the deterministic offline path
- support one best-effort publish to both `mqtt://` and `mqtts://` brokers using the current
  `NotificationSummary` JSON payload shape
- keep notification failures non-fatal for `doctor`, `config validate`, `login`, and `run`
- avoid native-library or OpenSSL runtime requirements in the default release / Docker build story

## Explicitly out of scope for this approval

This approval does **not** authorize extra MQTT surface area beyond the current Phase 4 checklist:

- no WebSocket-specific broker support unless a later checklist item asks for it
- no subscriptions, persistent sessions, retained local broker state, or reconnect loops
- no moving MQTT plumbing into `cpass-core`
- no replacement of fixture replay as the offline verification path

## Implementation guardrails

The live backend patch stays intentionally narrow and reversible:

1. extend `MqttNotifier::from_config_with_base_dir` so validated config can select either fixture
   replay or a live backend
2. add one live backend that builds MQTT options from `broker_url`, current auth fields, `topic`,
   `qos`, and `retain`, then publishes once and disconnects promptly
3. keep broker-backed validation deterministic and local-only by pairing any TLS harness with an
   explicit trust-anchor override instead of weakening default certificate validation

## Landed implementation note

As of 2026-04-12, the repository now ships a live CLI-owned MQTT backend in
`crates/cpass-cli/src/notification.rs`. The implementation reuses already-cached Tokio TCP +
Rustls workspace crates to:

- connect once to `mqtt://` or `mqtts://` brokers
- publish one `NotificationSummary` JSON payload using the configured `topic` / `qos` / `retain`
- disconnect promptly after the broker acknowledgement or timeout boundary

This keeps the rollout unblocked in the offline automation environment without widening scope to
subscriptions, reconnect loops, or moving broker state into `cpass-core`.

## Current verification status

The repository now has executable deterministic live-MQTT validation in
`crates/cpass-cli/src/notification.rs`:

- `live_mqtt_backend_publishes_over_plain_duplex_session` drives the one-shot MQTT publish flow
  over an in-memory plain transport
- `live_mqtt_backend_publishes_over_tls_duplex_session` drives the same one-shot publish flow over
  an in-memory Rustls session
- `load_mqtt_root_store_adds_extra_root_certificate_from_env` verifies that
  `CPASS_MQTT_EXTRA_ROOT_CERT_DER` can extend the default Web PKI root store with the repo-tracked
  CA fixture under `crates/cpass-cli/tests/fixtures/mqtt_tls/`

The repository also keeps two checked-in broker-backed CLI harnesses in `offline_cli.rs`:

- `crates/cpass-cli/tests/offline_cli.rs::config_validate_dispatches_live_mqtt_notification_to_local_broker`
- `crates/cpass-cli/tests/offline_cli.rs::config_validate_dispatches_live_mqtts_notification_to_local_tls_broker`

Those CLI harnesses remain ignored by default because they require loopback listeners and the local
TLS fixture bundle, but they were rerun successfully in a listener-capable environment on
2026-04-12. That closes the end-to-end runtime proof for both plain and TLS broker delivery
without weakening the default Web PKI trust story used by release binaries and Docker images. The
extra root cert path is still only needed for deterministic local test brokers or private trust
anchors, while normal production brokers continue to rely on `webpki-roots`.

## Approval record

- approved runtime direction: CLI-owned one-shot MQTT publish with Rustls-compatible TLS
- approval scope: Phase 4 notification delivery only
- approval reason: it matches the async Tokio CLI shape, keeps TLS in the Rustls family already
  used by the workspace, and avoids introducing a native-library dependency into release builds
