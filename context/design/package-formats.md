# Package Format Boundary

`ArtifactAdapter` is the only layer that understands source archive layout.

```rust
trait ArtifactAdapter {
    fn probe(&self, input: &Artifact) -> ProbeResult;
    fn read_metadata(&self, input: &Artifact) -> Result<NormalizedMetadata>;
    fn inspect_entries(&self, input: &Artifact) -> Result<EntryManifest>;
    fn extract_to(&self, input: &Artifact, staging: &Path) -> Result<ExtractionReport>;
}
```

Adapters must:
- preserve source metadata;
- reject malformed archives;
- never execute scripts;
- enforce extraction limits;
- expose scripts as data;
- expose package file metadata;
- distinguish config/ghost/symlink/hardlink where format supports it.

Format support is separate from compatibility support.
