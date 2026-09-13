# Repository Model

## Repository abstraction

```rust
trait Repository {
    async fn refresh(&self) -> Result<RepositorySnapshot>;
    fn candidates(&self, query: PackageQuery) -> Result<Vec<Candidate>>;
    fn artifact(&self, candidate: &Candidate) -> Result<ArtifactLocation>;
}
```

A repository snapshot is immutable after successful sync and identified by:
- repository ID;
- source URL/config;
- fetched-at timestamp;
- metadata digest;
- trust result;
- parsed normalized catalog.

## Repository types

- Debian repository adapter;
- RPM/DNF repository adapter;
- ALPM repository adapter;
- pkg-native index;
- direct URL/local file pseudo-repository.

## Rule

pkg may consume upstream repository metadata without mirroring all payloads. The local machine stores normalized indexes and downloads artifacts on demand.
