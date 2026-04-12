# MQTT TLS test fixtures

These DER files are test-only assets for the local `mqtts://` broker harness in
`crates/cpass-cli/tests/offline_cli.rs`.

- `ca.cert.der` — local CA/root certificate injected through `CPASS_MQTT_EXTRA_ROOT_CERT_DER`
- `server.cert.der` — localhost server certificate signed by that CA
- `server.key.der` — PKCS#8 private key for the localhost server certificate

They are deterministic offline integration-test fixtures only and must not be reused for real broker deployments.
