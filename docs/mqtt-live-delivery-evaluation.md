# MQTT Live Delivery Dependency Evaluation

This note narrows the remaining Phase 4 MQTT blocker to one concrete recommendation.
It does **not** approve a new dependency by itself; it records which runtime path best matches the
current `cpass-rs` CLI boundary so a future implementation task does not need to restart the crate
survey.

## Existing acceptance criteria

The live backend still has to satisfy the approval gate already captured in
[`docs/notification-pipeline.md`](./notification-pipeline.md):

- stay CLI-owned and keep broker state out of `cpass-core`
- publish the existing `NotificationSummary` payload to both `mqtt://` and `mqtts://` brokers
- keep release binaries and Docker images free of opaque native-library/runtime requirements
- preserve the current fixture-backed `MqttBackend` path for deterministic offline tests
- keep MQTT delivery best-effort so notification failures only warn and never retroactively fail the
  primary command

The current workspace also has one extra repository-level constraint: new dependencies are not added
casually, so the first follow-up after this note must still be an explicit approval to carry a real
MQTT client crate.

## Candidate options

### Option A — keep fixture replay only

**Pros**

- zero new dependencies
- current tests and config surface stay unchanged

**Cons**

- does not satisfy the remaining checklist goal of live broker delivery
- cannot validate `mqtts://` behavior at all

**Verdict**

Keep as the offline baseline, but it is not sufficient as the final Phase 4 runtime story.

### Option B — `rumqttc` with Rustls

**Fit**

- `rumqttc` exposes a Tokio-oriented `AsyncClient`/`EventLoop` model, which matches the existing
  async CLI runtime shape instead of forcing a second concurrency model
- the crate's published docs describe it as a pure-Rust MQTT client and show optional TLS plumbing
  through `tokio-rustls` / `rustls-*` as well as `tokio-native-tls`
- a Rustls-first setup lines up with the workspace's current `reqwest` + `rustls-tls` preference and
  avoids adding a new C or OpenSSL toolchain requirement to release builds by default

**Trade-offs to handle later**

- the event loop must be polled in the background, so the live backend should own a short-lived
  task that publishes one message and then disconnects cleanly
- feature selection needs to stay narrow; enabling both native TLS and Rustls by default would add
  unnecessary surface area
- broker-backed verification should remain opt-in/manual unless the repository later gains a stable
  local broker harness

**Verdict**

Recommended implementation path **if** the workspace explicitly approves one new MQTT dependency.
The best follow-up is to add `rumqttc` behind a CLI-only live backend that keeps fixture replay as
its offline test path and prefers Rustls for `mqtts://` support.

### Option C — `paho-mqtt` / Eclipse Paho wrapper

**Fit**

- feature-complete client with MQTT 3.1.1/5 support and TLS/WebSocket support

**Why it is not the default recommendation**

- the Rust crate is a wrapper around the Paho C library rather than a pure-Rust implementation
- the documented build flow pulls in bundled C sources and requires build tooling such as `CMake`
  and a C compiler; TLS-enabled variants also depend on OpenSSL handling
- that native toolchain/runtime story conflicts with the repository's current preference for
  portable Rust-first release and Docker builds

**Verdict**

Rejected for the default Phase 4 path unless the project explicitly chooses to accept native C / SSL
build requirements across every supported release target.

## Recommended approval decision

When the checklist reaches live MQTT delivery again, prefer this approval package:

1. approve a CLI-only `rumqttc` dependency with a Rustls-first feature set
2. keep `fixture_response_path` support unchanged as the deterministic offline backend
3. scope the first live implementation to one best-effort publish + disconnect flow for
   `doctor`, `config validate`, `login`, and `run`
4. defer WebSocket-specific brokers, persistent sessions, and subscription support unless a later
   checklist item requires them

## Implementation outline after approval

Once the dependency is explicitly approved, the next coding task should be:

1. extend `MqttNotifier::from_config_with_base_dir` so it can choose either fixture replay or a live
   backend from the same validated config
2. add a `LiveMqttBackend` in `crates/cpass-cli/src/notification.rs` (or a focused helper module)
   that:
   - parses `broker_url`
   - configures one `rumqttc::MqttOptions`
   - maps `mqtt://` to plain TCP and `mqtts://` to Rustls-backed TLS
   - publishes the existing JSON payload with current `topic` / `qos` / `retain`
   - disconnects promptly after the publish acknowledgement or timeout boundary
3. keep all current fixture-backed tests, then add non-network unit coverage around request/options
   building before deciding whether any broker-backed integration test belongs in-repo

## Sources

- [`rumqttc` docs.rs crate page / TLS source](https://docs.rs/rumqttc/latest/src/rumqttc/tls.rs.html)
- [`rumqttc::AsyncClient` docs](https://docs.rs/rumqttc/latest/rumqttc/struct.AsyncClient.html)
- [Eclipse Paho MQTT Rust client README](https://github.com/eclipse-paho/paho.mqtt.rust)
