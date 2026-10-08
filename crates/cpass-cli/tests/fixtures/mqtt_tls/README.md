# MQTT TLS broker test fixtures

Test-only self-signed localhost certificate + PKCS#8 key for the offline `mqtts://`
broker harness in `crates/cpass-cli/tests/offline_cli.rs`.

The same DER certificate is used both as the server certificate and as the extra test root
passed through `CPASS_MQTT_EXTRA_ROOT_CERT_DER`.
