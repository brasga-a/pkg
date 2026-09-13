# Observability

Use structured tracing around:
- repository refresh;
- candidate selection;
- artifact download;
- verification;
- compatibility checks;
- transaction phases;
- recovery;
- activation.

Correlation IDs:
- transaction ID;
- repository snapshot ID;
- artifact digest;
- package ID.

Logs are diagnostic and never determine package correctness.

Provide `pkg doctor` for:
- state/store consistency;
- dangling activation;
- incomplete transaction;
- missing host capability;
- corrupted cache entry.
