# Package Model

## NormalizedPackage

Conceptual shape:

```rust
struct NormalizedPackage {
    id: PackageId,
    name: PackageName,
    version: SourceVersion,
    architecture: Architecture,
    source_format: PackageFormat,
    artifact: ArtifactIdentity,
    dependencies: Vec<DependencyExpr>,
    provides: Vec<Capability>,
    conflicts: Vec<CapabilityExpr>,
    files: Vec<PackageEntry>,
    scripts: Vec<LifecycleScript>,
    integrations: Vec<IntegrationHint>,
    original_metadata: RawMetadataRef,
}
```

Normalization does **not** discard source semantics. Each dependency keeps:
- original expression;
- source ecosystem;
- parsed comparator/version;
- normalized capability candidates where known.

A package may expose multiple identities:
- logical package name;
- executable capabilities;
- ELF SONAME provides;
- virtual provides;
- pkg-native aliases.

## Package classes

- `PortableBinary`
- `PortableDesktopApp`
- `RelocatableTree`
- `NeedsHostLibraries`
- `NeedsHostIntegration`
- `ContainerPreferred`
- `SystemCritical`
- `Unsupported`

Classification can change as analyzers improve, but security policy remains default-deny for dangerous classes.
