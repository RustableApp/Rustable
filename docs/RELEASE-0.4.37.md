# Rustable v0.4.37

- Redis and Valkey now offer explicit **Verify full**, **Require TLS (unverified)**, and **Disabled (unencrypted)** modes. Choosing unverified TLS keeps encryption, SNI and client certificates; it no longer accidentally selects plaintext. Existing saved connections keep their previous security settings.
- Local port-forward mappings let you connect through 127.0.0.1 while retaining the real database hostname for TLS, without editing `/etc/hosts`. Add mappings for individual Cluster or Sentinel endpoints as needed.
- Sentinel discovery and primary connections keep independent TLS policies. Connection-test results report those policies accurately, and connection errors include the selected TLS mode and troubleshooting guidance.

Fixes [#17](https://github.com/RustableApp/Rustable/issues/17).
