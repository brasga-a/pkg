# Integrity and Signatures

## Integrity layers

1. transport security;
2. repository metadata authenticity;
3. artifact digest integrity;
4. artifact/package signature;
5. local store identity.

These are not interchangeable.

## Trust policy

Repository adapters expose verification evidence. The pkg core decides whether policy accepts it.

```text
Trusted
TrustedWithWarning
UnverifiedAllowedByPolicy
Rejected
```

Default remote repository behavior should require authenticated metadata and validated artifact digest.

## Native trust systems

Debian, RPM and ALPM ecosystems use different signing/key models. Preserve their evidence rather than translating all signatures into a fictitious universal signature.

A future pkg-native repository may use a simpler dedicated signed-index scheme.

## Key rotation

Trust roots are configuration/state with explicit identity and rotation history. A repository payload must never be allowed to silently install a new root key.
