# Rust Ecosystem Recommendations

Versions change; this file records architectural candidates, not pinned dependency versions.

## CLI
- `clap` — mature command parser.

## Async/network
- `tokio` — async runtime.
- `reqwest` — HTTP client with TLS/proxy/redirect support.

## Serialization/config
- `serde`
- `toml`
- `serde_json` for repository formats/debugging where applicable.

## Errors
- `thiserror` for domain/library errors.
- `anyhow` only at application/CLI edges.

## Logging
- `tracing`
- `tracing-subscriber`

## Database
- `rusqlite` for a local synchronous state DB is the simplest fit.
- `sqlx` is viable if async DB access or compile-time query workflow becomes valuable, but SQLite operations are not the system bottleneck.

## Package/archive parsing
- RPM: `rpm` crate is a strong candidate.
- Debian: use `ar` + `tar` + compression crates or a well-maintained dedicated parser after evaluation.
- ALPM: `tar` + `zstd` plus explicit `.PKGINFO` parser is straightforward; evaluate dedicated ALPM crates before adding one.

## Compression
- `flate2`
- `xz2` or maintained liblzma binding
- `zstd`
- `bzip2` only where required.

## Binary inspection
- `goblin` for ELF parsing.

## Hashes/signatures
- `sha2`, `blake3` where appropriate.
- `ed25519-dalek` or `minisign` for a future pkg-native trust scheme; source-native signatures should use ecosystem-compatible verification.

## Dependency solving
- `pubgrub` is a candidate after the normalized constraint IR is proven.
- Do not couple the domain model directly to its types.

## Testing
- `proptest`
- `tempfile`
- `assert_cmd`
- `predicates`
- fixture archives generated in tests.
