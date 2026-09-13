# Terminology

**Artifact** — downloaded/local package file or archive.

**Source format** — Debian `.deb`, RPM, ALPM `.pkg.tar.zst`, generic tarball, etc.

**NormalizedPackage** — format-neutral metadata representation produced by an adapter.

**Capability** — a requirement/provision that may represent a package, executable, shared library/SONAME, ABI feature or virtual feature.

**Host fact** — observed property of the machine: architecture, libc, available SONAME, kernel, executable, filesystem capability.

**Store object** — pkg-owned installed package tree identified by package/version/build/digest.

**Activation** — link/shim/profile record exposing content from a store object.

**Profile** — selected set of active package versions for a user/environment.

**Catalog** — normalized view over one or more repositories.

**Repository adapter** — parser/fetcher for Debian, RPM, ALPM or pkg-native repository metadata.

**Install plan** — immutable planned actions and compatibility findings before side effects.

**Transaction** — durable attempt to apply an install/update/remove plan.

**Native provider** — optional integration that asks the host package manager to satisfy a capability; never implicit in MVP.

**Maintainer script** — lifecycle script embedded in a distribution package.

**Relocatable** — package can operate from pkg store without assuming original absolute installation paths.

**Portable class** — policy classification describing how safely a package can be cross-installed.
