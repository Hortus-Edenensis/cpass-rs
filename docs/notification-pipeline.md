# Notification Pipeline

Phase 4 notification work stays on the CLI side of the boundary.

`cpass-core` owns only two pieces today:

- `NotificationConfig` loading and validation in `crates/cpass-core/src/config.rs`
- the shared `RunEvent` / `RunEventSink` lifecycle contract in `crates/cpass-core/src/event.rs`

Everything else stays in `crates/cpass-cli`:

- `notification.rs` fans one `RunEvent` stream out to multiple CLI-owned sinks
- `NotificationSummaryCollector` turns terminal lifecycle events plus warnings/retries into one
  backend-agnostic `NotificationSummary`
- `NotificationCoordinator` pairs that summary with each validated `notifications[*]` entry and
  yields ready-to-send `NotificationPlan` values for later Gotify or MQTT backends
- `dispatch_notification_plans` now delivers both the supported Gotify plans and the current
  fixture-backed-or-live MQTT plans without pushing backend concerns into `cpass-core`

## Current Scope

The current implementation now sends best-effort Gotify and MQTT notifications from the CLI side
for the commands that already end in a terminal summary:

- `cpass doctor`
- `cpass config validate`
- `cpass login`
- `cpass run`

MQTT stays intentionally scoped to the same CLI-owned summary flow. The dispatcher still resolves
`fixture_response_path` relative to the config file directory for deterministic offline replay, but
it can now also open one live broker connection, publish the same stable JSON payload, and
disconnect promptly without widening the runner boundary.

The notifier-side parser now normalizes MQTT config into one shared validated shape inside
`crates/cpass-cli`:

- publish settings: `topic`, `qos`, and `retain`
- backend selection: either fixture replay or a live broker target
- live-target details: validated `broker_url`, resolved host/port + transport (`mqtt` vs `mqtts`),
  plus optional `client_id` / `username` / `password`

That keeps fixture replay and live delivery on one shared config path instead of forcing another
config migration before real broker delivery lands. The current CLI keeps the same failure
boundary: live broker delivery is best-effort and only emits a warning when publish/connect fails,
while the validated config remains visible through `config validate` and the rest of the CLI-owned
notification flow.

The delivery boundary still stays the same:

- `login` can build a success summary from `login_succeeded`
- `cpass run` can build a completion or blocked summary from `course_run_execution_finished`
- warnings and retry scheduling stay attached to the same summary without pushing notification
  state into `cpass-core`

That keeps the runners, queue driver, and executors unaware of notification transport details.
Future delivery backends should keep consuming the CLI-owned `NotificationPlan` output instead of
adding Gotify or MQTT concerns to `cpass-core`.

## Gotify Contract

The CLI sends one `POST` request per Gotify notification config to the configured `base_url`, with
the app token appended as the `token` query parameter. The JSON request body is intentionally plain
text and stable:

- `title`: summary title from the terminal `RunEvent`
- `message`: a text body containing `kind`, `level`, summary body text, retry counts, and any
  accumulated warnings
- `priority`: the configured integer priority, defaulting to `0` when omitted
- `extras.client::display.contentType`: always `text/plain`

Delivery is best-effort. If the main command succeeds but Gotify delivery fails, the CLI keeps the
primary command result and prints a warning to stderr instead of retroactively failing the main
operation.

## Fixture Replay

Offline tests can pin Gotify delivery with a local response fixture:

```yaml
notifications:
  - type: gotify
    base_url: "https://gotify.example/message"
    token: "demo-token"
    priority: 5
    fixture_response_path: "gotify_message_response.json"
```

When `fixture_response_path` is present, the CLI resolves it relative to the config file directory
and replays that JSON response instead of issuing a live HTTP request. This keeps offline CLI tests
deterministic while still exercising the same request-building path as the real backend.

## MQTT Fixture Contract

The MQTT notifier contract now serializes one JSON payload per terminal `NotificationSummary`:

- `schema`: fixed `cpass.notification.v1`
- `kind`, `level`, `title`, `body`: copied from the terminal summary
- `warning_count`, `retry_count`: scalar summary counters
- `warnings`: ordered warning strings gathered during the command run

Fixture-backed tests can pin the same contract with a local response fixture:

```yaml
notifications:
  - type: mqtt
    broker_url: "mqtt://broker.example:1883"
    topic: "cpass/status"
    qos: 1
    retain: true
    fixture_response_path: "mqtt_publish_response.json"
```

The fixture JSON is intentionally small and transport-agnostic. The current notifier and CLI
dispatcher only check the recorded acknowledgement shape needed by tests:

```json
{
  "status": "accepted",
  "topic": "cpass/status",
  "qos": 1,
  "retain": true
}
```

Delivery is still best-effort: if MQTT fixture replay fails, the main command result stays intact
and the CLI prints a warning to stderr, matching the existing Gotify behavior.

## Live MQTT Delivery

When `fixture_response_path` is omitted, the CLI now treats the MQTT notification as a live broker
target instead of a fixture replay:

- `mqtt://` uses one plain TCP connection to the configured host/port
- `mqtts://` uses the same one-shot publish flow with a Rustls client config rooted in
  `webpki-roots`, optionally extended via `CPASS_MQTT_EXTRA_ROOT_CERT_DER` for deterministic
  local/private broker trust anchors
- optional `client_id`, `username`, and `password` stay CLI-owned and never leak into
  `cpass-core`
- after one publish acknowledgement (or timeout / broker rejection), the CLI disconnects promptly

This keeps live delivery narrow and reversible: no subscriptions, reconnect loops, WebSocket
support, or persistent broker session state were added as part of this task.

## Live MQTT Delivery Approval Gate

The workspace keeps the rollout guardrails recorded in
[`docs/mqtt-live-delivery-approval.md`](./mqtt-live-delivery-approval.md). The landed live backend
now satisfies the transport boundary, and the deterministic validation that runs by default in
automation lives in `crates/cpass-cli/src/notification.rs`
(`live_mqtt_backend_publishes_over_plain_duplex_session`,
`live_mqtt_backend_publishes_over_tls_duplex_session`, and
`load_mqtt_root_store_adds_extra_root_certificate_from_env`). The checked-in broker-backed CLI
harnesses in `crates/cpass-cli/tests/offline_cli.rs`
(`config_validate_dispatches_live_mqtt_notification_to_local_broker` and
`config_validate_dispatches_live_mqtts_notification_to_local_tls_broker`) remain ignored by
default because they require loopback listeners, but they were rerun successfully in a
listener-capable environment on 2026-04-12 to close the end-to-end broker proof for both
`mqtt://` and `mqtts://`.

The notifier-side config parsing preserves the validated broker URL plus optional MQTT auth /
client-id fields whether `fixture_response_path` is present or absent, so fixture replay and live
delivery continue to share one config path.

The live backend currently satisfies all of the following:

- support the existing async CLI execution model without pushing broker state into `cpass-core`
- publish one best-effort notification payload to both `mqtt://` and `mqtts://` broker URLs
- provide a TLS story that is acceptable for release binaries and Docker images without introducing
  an opaque native-library/runtime requirement
- fit behind the current `MqttBackend` trait so fixture replay and offline tests keep their current
  deterministic path
- preserve the current failure boundary: notification delivery may warn, but it must not retroactively
  fail an otherwise successful `doctor`, `config validate`, `login`, or `run` command

The deterministic `mqtts://` path uses the repo-tracked CA/server certificate fixture bundle under
`crates/cpass-cli/tests/fixtures/mqtt_tls/` plus `CPASS_MQTT_EXTRA_ROOT_CERT_DER` so the CLI can
trust the local test broker without weakening the default Web PKI root set.

`fixture_response_path` still remains the preferred deterministic offline path for tests and
fixture-backed automation, even though live delivery is now supported, the in-memory plain/TLS
unit coverage runs in the sandbox by default, and the checked-in broker-backed harnesses have
already covered both `mqtt://` and `mqtts://` in the 2026-04-12 listener-capable rerun.

The earlier dependency comparison and recommendation still live in
[`docs/mqtt-live-delivery-evaluation.md`](./mqtt-live-delivery-evaluation.md), while the explicit
workspace guardrails and landed implementation note now live in
[`docs/mqtt-live-delivery-approval.md`](./mqtt-live-delivery-approval.md).
