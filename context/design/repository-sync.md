# Repository Sync

## Flow

```text
configured repo
   |
fetch metadata
   |
transport checks
   |
signature/trust verification
   |
parse source metadata
   |
normalize candidates
   |
write new snapshot
   |
atomic active-snapshot switch
```

A failed refresh leaves the previous valid snapshot active.

## Mirrors

Mirror selection is repository-specific but normalized into endpoint candidates. Record:
- endpoint;
- last success/failure;
- latency;
- metadata identity.

Never accept mismatched metadata merely because a mirror is responsive.

## Freshness

Repository freshness policy is explicit. Offline mode may use stale metadata with a clear diagnostic.

## Cache

Metadata and payload caches are distinct:
- metadata cache keyed by repo snapshot identity;
- artifact cache keyed by cryptographic digest when known.
