# Local repository fixture

`InRelease` authenticates the compressed `Packages.gz` and `Packages.xz` bytes.
`test-key.asc` is the public part of rPGP 0.14.2's
`tests/rfc9580/v4-ed25519-x25519/tsk.asc` test vector, not a production trust anchor.
The fixture signature was generated once with `CleartextSignedMessage::new`;
no private key or GPG executable is required
when running the tests. Tests serve these files over loopback and alter them to
verify rejection, without accessing external repositories or host keyrings.

`Packages-tampered.gz` and `Packages-tampered.xz` are valid compressed indexes
containing `forged-tool` instead of `review-tool`. Their hashes are not authorized
by InRelease, so tests distinguish authentication failures from broken codecs.
