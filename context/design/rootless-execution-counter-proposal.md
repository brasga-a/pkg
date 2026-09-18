# Rootless Execution: Verified Environments per Command

Date: 2026-09-16

Status: **counter-proposal / implementation in progress / acceptance gates pending**

Responds to: [Rootless Execution Model](rootless-execution-model.md)

## 1. Decision proposed

Keep the isolated store, rootless installation, format adapters and default-deny lifecycle policy. Introduce an explicit execution contract for each exposed command: its executable or interpreter, exact dependency providers, supported runtime behavior, adaptations and evidence of compatibility.

An installed package is a stored artifact realization. An executable command is an activation of a verified environment that references one or more realizations. These have separate identities and lifetimes.

The format of a package determines how it is inspected. It does not determine whether a library can satisfy another package's requirement. This applies equally to Debian `.deb`, RPM `.rpm` and ALPM artifacts such as `.pkg.tar.zst`. This document uses **ALPM** as the ecosystem name; `.alpm` is not a proposed filename extension or new format detector.

This proposal does not supersede accepted ADRs, change command behavior, or mark any milestone complete. Its authority remains below accepted decisions and the [architectural invariants](../project/invariants.md). Acceptance requires reconciling the documents listed in section 14.

## 2. What changes from the original proposal

| Topic | Original proposal | Counter-proposal |
|---|---|---|
| Support claim | Five adaptation layers describe rootless execution generally | A support matrix admits specific package/runtime/host combinations; unresolved requirements block activation |
| Shared libraries | Every relevant launcher searches `profiles/<name>/lib` | Each command references a selected, verified dependency closure; no profile-wide library search directory |
| Launch strategy | Decide from static/dynamic classification and existence of profile directories | Decide from the command's execution contract, including its interpreter and child-process requirements |
| Runtime variables | Infer environment settings from directory names | Directory discovery supplies candidates to a reviewed runtime adapter |
| Relocation | Bundled FHS paths authorize substitutions | A versioned recipe authorizes each transformation and records its inputs and outputs |
| Configuration | Infer defaults from `.ucf` / `.default` and compensate for scripts | Materialize explicitly mapped defaults for supported cases; classify remaining lifecycle requirements |
| Integrity | Describe payloads as pristine while transforming them | Preserve source artifact identity separately from derivation identity and realized file digests |
| Publication | Promote, create library links, then expose commands | Prepare all objects first; publish an activation generation with a recoverable filesystem/DB protocol |
| Completion | Checked invariant list | Unchecked acceptance gates tied to implementation and execution tests |

## 3. Current implementation baseline

The following is a source inspection snapshot, not an execution certification. Future implementation work must refresh it against the checkout being changed.

| Area | Observed behavior | Missing contract |
|---|---|---|
| [Planner](../../crates/pkg-core/src/planner/mod.rs) | Produces normalized package plans, dependency closure and derivation identity | Complete support matrix and pure-plan evidence for every input class |
| [ELF inspection](../../crates/pkg-core/src/host/elf.rs) | Resolves exact paths, transitive `DT_NEEDED`, interpreter, machine/class and symbol versions | Full libc/CPU matrix and explicit-loader adapter evidence |
| [Runtime preparation](../../crates/pkg-core/src/runtime/mod.rs) | Generates command-local Python adapters, pure Perl/Node adapters and static native ELF launchers, records bootstrap/configuration evidence, then revalidates providers at launch | Verified R/Ruby adapters, native interpreter extensions and full host matrix |
| [Library activation](../../crates/pkg-core/src/activation/mod.rs) | Keeps the legacy flat `profile/lib` compatibility links while runtime manifests persist exact consumer-to-provider roots | Removal of the legacy flat path and complete helper/plugin provider policy |
| [Text relocation](../../crates/pkg-core/src/domain/relocation.rs) | Rewrites reviewed text paths and materializes only an exact sibling `.ucf` default, recording generated outputs | Mutable user configuration overlay and broader reviewed recipes |
| [State database](../../crates/pkg-core/src/state/db.rs) | Records artifact/store identity, runtime manifests, generations, receipts and closure references | Lease/retention policy for running processes |
| [Install executor](../../crates/pkg-core/src/engine.rs) | Prepares and inventories transformed payloads, promotes the tree, materializes runtime views and publishes generations | Fault-injection corpus across every filesystem/SQLite boundary |

Existing atomic replacement of an individual executable link is useful. It is not an atomic switch of all commands, libraries and database rows in an installation.

The [dependency reuse report](../../report.md) and [install implementation plan](../../docs/install-implementation-plan.md) describe related gaps. Neither the original document nor this counter-proposal proves they have been closed.

## 4. Support boundary and compatibility outcomes

Rootless execution here means application installation into user-owned locations without native package database mutation. It does not provide a filesystem namespace or an application sandbox: an executed program retains the user's ordinary permissions.

The initial supported subset is:

- self-contained commands whose required files work from the store;
- dynamic commands compatible with an explicitly supported host loader/libc and a verified library closure;
- scripts supported by a reviewed interpreter adapter;
- applications whose required configuration can be handled through explicit, deterministic adaptations.

Compiled absolute file paths, runtime-generated plugin names, arbitrary subprocess behavior and lifecycle effects cannot be solved universally by setting environment variables. Required behavior outside an adapter's supported contract returns `Unsupported`, `NeedsHostIntegration`, `Unknown` or `Unsatisfied`, with the requirement chain. Unknown is not success. An optional unsupported feature may be omitted only when the package's support recipe identifies it as optional and the plan reports the omission.

Follow the outcomes in the [compatibility model](compatibility-model.md): each requirement records `SatisfiedByClosure`, `SatisfiedByHost`, `NeedsNativeProvider`, `NeedsIntegration`, `Unknown` or `Unsatisfied`. Only satisfied requirements and explicitly omitted optional features permit a verified activation.

System-critical replacements, foreign libc/loader bundles, privileged services, kernel/init/PAM changes and arbitrary maintainer scripts remain outside the default policy. A source-distro runtime or container fallback is a separate future design. A static executable is not automatically self-contained: configuration, helper programs and data requirements still apply.

## 5. Data model and ownership

These are proposed internal concepts, not additions to the stable JSON or MCP schema.

| Concept | Required content |
|---|---|
| `ArtifactEvidence` | Source, ecosystem, architecture, digest/size, catalog snapshot identity and applicable authenticity evidence; local unsigned provenance remains distinguishable |
| `PayloadManifest` | Normalized entries, file digests, types, modes, links, ELF/script facts, lifecycle inventory and inspection completeness |
| `AdaptationPlan` | Recipe identifier/version, matched inputs, exact outputs, preconditions and semantic postconditions |
| `ExecutionPlan` | Command, executable/interpreter, ordered argv, environment policy, selected providers, child/plugin contract and explanation chain |
| `ProviderEvidence` | Consumer requirement, concrete provider file, owning realization or host identity, ABI facts and reason for selection |
| `RuntimeManifest` | Frozen execution plan, verification status, command-specific library/module views, runner version, host evidence and references to all required objects |
| `ActivationGeneration` | Complete exposed command set, runtime references, owned paths and previous generation |
| `TransactionReceipt` | Frozen plan identity, old/new generation, newly created versus reused objects, expected digests and durable phase history |

The **dependency closure** is the graph of all providers required by a command, including transitive requirements and the chosen interpreter when present. It may include host providers and package providers. A package declaring a dependency does not itself become a provider of that dependency.

Provider references use object identities and relative paths, never an unqualified package name or “the latest library in this profile.” Metadata names and version constraints retain their source ecosystem semantics. Cross-ecosystem equivalence requires an explicitly supported capability constraint and evidence.

### Proposed layout

`<pkg-root>` denotes the configured root; this is a target extension of the [store layout](store-layout.md), not the current on-disk schema.

```text
<pkg-root>/
  cache/artifacts/<digest>                  original verified artifact
  staging/<transaction>/                   unpublished payload/runtime/generation work
  store/<realization-id>/                  installed payload, immutable after promotion
  runtimes/<runtime-id>/
    manifest                              frozen execution contract
    lib/                                  selected aliases to provider files
    modules/                              adapter-specific views when required
  profiles/<name>/
    generations/<generation-id>/
      manifest
      bin/<command>                       versioned native entrypoint or proven direct link
    current -> generations/<generation-id>
    bin -> current/bin
  state/pkg.db                            logical state and transaction records
```

There is no new global `profiles/<name>/lib` search path. Runtime views are immutable references, not additional payload copies. Two commands may share the same runtime object only when the entire relevant execution contract is identical.

An entrypoint resolves its generation once and pins the corresponding runtime manifest. It must not repeatedly resolve `current` while finding libraries or configuration. Runner implementations are versioned and retained for the manifests that use them.

## 6. Acquisition, pure planning and installation

The simplified installation drawing is useful for orientation:

```text
source/catalog -> artifact acquisition -> verified inputs
                                              |
                                    pure compatibility plan
                                              |
                                    transaction staging
                                              |
                                  verified store/runtime objects
                                              |
                                  profile generation publication
                                              |
                                    installed-state commit
```

The state database also journals transaction phases before publication; it is not written only at the final box. Cache storage and repository snapshots have their own acquisition/synchronization lifecycle.

### Pure planning

The planner consumes verified artifact views, fixed catalog/state/host snapshots and policy. It returns a complete plan or an explanation of missing evidence. It does not download, create temporary files, initialize SQLite, acquire a writer lock, or create staging/profile directories.

Inspect available artifacts using bounded read-only archive views and static binary parsing. If more artifacts are needed, return acquisition requirements to the application/executor boundary. That boundary can fetch and verify them, then invoke planning again. Bound the number of acquisition rounds, total bytes and graph size; cycles in the dependency graph must not cause unbounded downloads or recursion.

The final plan includes all payload changes, generated files, provider edges, runtime manifests, activation changes, owned integration actions and recovery expectations. Staging validates and materializes these decisions; it does not silently discover and apply a new recipe.

`--dry-run` uses the same pure planner. When required bytes are unavailable, it returns an incomplete preview with explicit missing evidence and a non-success diagnostic for a requested verified plan. It does not report compatibility based only on metadata. The [install implementation plan](../../docs/install-implementation-plan.md) follows this no-acquisition boundary; formal acceptance still requires aligning the canonical command contract and ADRs.

### Execution and publication

For one requested target, its newly required package closure and runtime objects form one recoverable operation. Several independent CLI targets need not share a transaction unless a separate command contract promises that behavior.

1. Acquire/verify required artifacts through the acquisition boundary. Integrity and authenticity remain separate findings.
2. Produce the complete plan from immutable inputs.
3. Acquire the pkg writer lock, reconcile interrupted transactions and revalidate state, host evidence, artifact identity and destination ownership. If assumptions changed, fail or replan before materialization.
4. Durably record the transaction and old/new generation intentions before package-state mutation.
5. Extract into transaction-owned staging, apply the exact recipes, generate runtime views/entrypoints and verify all realized outputs against the plan. Any required copy or link failure aborts preparation.
6. Promote verified payload/runtime objects on the same filesystem using atomic rename. Reuse an existing object only after checking its complete identity and expected manifest. Record which objects this transaction created.
7. Prepare and durably persist the complete activation generation, then atomically switch `current`. Check existing ownership before the switch; unmanaged commands remain conflicts.
8. Commit installed-state references, ownership records and provider edges in SQLite, then mark the journal complete.

Atomic rename does not make filesystem changes and SQLite one transaction. The [transaction model](transaction-model.md) still requires recovery evidence and appropriate durability ordering. Success is reported only after both activation and installed-state commit are confirmed.

| Interruption point | Recovery action |
|---|---|
| Before object promotion | Remove only staging owned by this transaction; retain verified cache entries |
| Objects promoted, profile unchanged | Preserve reused objects; remove or retain newly created unreferenced objects according to the receipt |
| Profile switched, DB not committed | Validate the new generation and finish its commit when evidence is complete; otherwise restore the recorded previous generation, then reconcile state |
| DB committed, completion marker missing | Confirm generation/DB agreement and mark complete idempotently |
| Files, pointer or ownership disagree with the receipt | Stop automatic mutation and report recovery required; never infer permission to delete an unknown path |

The generation switch governs future launches. It cannot atomically change already running applications or external host integrations. Desktop/service integration remains outside this initial publication unit.

## 7. ELF resolution and launch policy

### Provider selection

For every command and selected shared object, inspect ELF architecture/class/endianness, interpreter requirements, `DT_NEEDED`, SONAME, RPATH/RUNPATH and required/provided symbol versions. Include supported CPU/libc requirements. Resolve transitive edges, record the exact provider and reject missing or unsupported facts.

SONAME and symbol-version checks are necessary evidence, not a proof of all application semantics. Published support remains limited to tested combinations and supported adapter contracts.

Selection is deterministic:

1. Honor admissible explicit provider constraints and package-local providers that the supported loader will actually select.
2. Evaluate compatible existing store and host candidates under policy; a valid already selected provider is reusable across formats.
3. Acquire a new provider only when required evidence or capabilities are missing from admissible existing candidates.
4. Use source/repository preferences only to order candidates that already satisfy requirements and trust policy.
5. Record why a candidate was accepted or rejected. Never fall back to approximate package-name matching after a capability failure.

A runtime view maps each loader-visible name to one selected provider file and records all required aliases. It does not expose entire unrelated library directories. If a single process needs incompatible providers for the same effective library identity, reject that combination; putting both directories on a search path is not a solution. Distinct commands can use different providers in distinct runtime views.

Package-private library layouts using origin-relative resources require an adapter that preserves and tests that layout. Flattening a library and losing its adjacent resources is not an admissible generic adaptation.

### Loader behavior

On the supported glibc path, a dependency containing `/` is a pathname. Otherwise the relevant order is `DT_RPATH` when `DT_RUNPATH` is absent, `LD_LIBRARY_PATH`, `DT_RUNPATH`, the loader cache and default directories. `RUNPATH` applies to direct dependencies; `RPATH` has different transitive behavior. Secure execution can ignore environment overrides. The kernel uses the ELF interpreter request; a library search path cannot repair an incompatible interpreter. [Linux `ld.so` documentation](https://man7.org/linux/man-pages/man8/ld.so.8.html).

The verifier must model the selected adapter's actual behavior, including supported origin expansion, cache/hardware variants and transitive lookups. Merely finding a file in `extra_search_dirs` is insufficient. A name that would select a different provider at launch fails verification.

### Launch strategies

| Strategy | Admission rule |
|---|---|
| Direct link | Statically linked, proven self-contained command with no required adaptation or managed runtime checks |
| Native runner, normal execution | A verified host-compatible command/interpreter; runner applies the declared environment policy and validates host evidence |
| Native runner, explicit host loader | Supported glibc combination needing command-local package libraries; loader invocation and application behavior covered by fixtures |
| Interpreter adapter | Verified interpreter version, arguments, module/native-extension closure and any child-process requirements |
| Unsupported | No strategy can satisfy the execution contract |

For the explicit-loader strategy, use the verified host loader with its supported `--library-path` option pointing to the runtime view, and `--argv0` when required and supported. These are loader-specific facilities, not a POSIX guarantee. Do not bundle a foreign libc to make this strategy work. [Linux `ld.so` options](https://man7.org/linux/man-pages/man8/ld.so.8.html).

Explicit loader invocation is a proposed adapter choice, not transparent emulation. Test argument identity, executable-path discovery, origin-relative lookup, signals and re-execution. Reject packages that depend on invocation behavior the adapter cannot preserve. No default `patchelf` pass is introduced; binary transformation remains subject to the [relocation policy](filesystem-layout.md) and an accepted ADR.

Use a native Rust runner with structured argv/environment operations. Package metadata is never interpolated into a shell command. The runner must preserve arguments, exit status and supported signal behavior. Paths that cannot be represented safely in an adapter's path-list syntax are rejected, not escaped by guessing.

The initial runner bootstrap is a self-contained static build for each supported architecture. A dynamically linked runner cannot sanitize loader variables before its own loader has used them. Do not depend on package/profile libraries to start the component responsible for selecting those libraries.

### Environment, children and host changes

- No global shell configuration is changed. Runtime-managed search variables are set to the contract's values; arbitrary inherited search paths are not appended.
- The verified mode neutralizes ambient loader overrides such as `LD_LIBRARY_PATH`, `LD_PRELOAD` and `LD_AUDIT`. Each adapter defines its other managed variables and supported inheritance policy. Ordinary application variables remain available unless the adapter declares a reason to handle them.
- A command-local loader invocation does not establish a library environment for a later child `exec`. Required helper commands, module loads and plugin paths must be explicitly covered by an adapter or remain unsupported. Do not solve child execution by exporting all profile libraries to every descendant.
- Host providers are external, mutable dependencies. Record their concrete paths and file/ABI evidence; revalidate before activation and through the native runner when host files change. Invalid evidence requires replanning, not a silent alternate provider.
- pkg does not lock native package managers. Host changes racing with execution are an explicit limit of the host-provider strategy; this design does not promise a hermetic host runtime.

Production inspection statically parses artifacts. It must not run `ldd`, execute the target, or invoke a loader on untrusted payloads to establish compatibility. Controlled execution tests use repository-owned fixtures.

## 8. Runtime adapters and lifecycle requirements

Directory names discover potential runtime inputs; they do not establish compatibility by themselves.

| Runtime/class | Evidence required before admission |
|---|---|
| Python | Compatible interpreter and shebang arguments, bounded module roots, native-extension ABI and module dependencies; no automatic Python 2-to-3 or incompatible minor-version substitution |
| R | Verified R executable/runtime relationship, explicit home/config mapping and native package requirements |
| Perl, Ruby, Node | Interpreter/package layout and any native-module ABI verified by a dedicated adapter |
| Desktop | Explicit desktop/data/schema/plugin capabilities; `XDG_DATA_DIRS` alone is insufficient |
| Shell/helper programs | Preserved interpreter arguments plus an explicit contract for required executable paths and environment |

Implement Python first using the existing fixture work as a starting point. Pure Perl and Node scripts may use the generic absolute-interpreter adapter when the host executable is present; native extensions and R, Ruby and desktop support remain individually gated. Finding `usr/lib/R` or a module directory never enables an unimplemented adapter.

Inventory maintainer scripts as metadata. A reviewed support recipe states which required effects are already satisfied by the payload, replaced by an admissible adaptation, or require unsupported integration. Do not attempt to prove arbitrary shell script semantics automatically. Unclassified required behavior prevents verified activation; script presence alone is neither blanket rejection nor proof that omission is harmless.

## 9. Relocation, links and configuration

`PackageFhsIndex` remains useful for discovery and checking that a referenced component exists. It is not sufficient authorization to rewrite every matching string in a text file.

Each reviewed recipe identifies package inputs by verified identity or constrained metadata plus checked file content. It specifies exact source/destination paths, supported syntax, expected digests or structural preconditions, output ownership and validation. Transformations run in a fixed order and appear in the install plan.

1. Validate archive entries and topology before following links or writing through them.
2. Materialize only explicitly mapped default files; missing inputs, ambiguous destinations and write failures are errors.
3. Apply approved text transformations to original and generated files.
4. Validate the final link graph and file manifest, then verify runtime lookup against the resulting layout.

Absolute symlinks may be relativized only when their meaning is known to be internal to that payload or to a file generated by its recipe. A required link into host `/etc` or another package is not silently redirected to a guessed local file. Cross-package references belong in the execution/integration plan. Required dangling links, cycles and escaping chains fail closed. Validate hardlinks and extraction through parent symlinks as well as textual link targets.

For a supported R case, a recipe could map `usr/lib/R/etc/Renviron.ucf` to `etc/R/Renviron`. It must verify that this is the intended source/destination pair and that the runtime actually reads the adapted path. The `.ucf` suffix does not authorize copying that file into every discovered `etc/<name>` directory. `.default` has no generic meaning in this policy.

Read-only defaults can live in the store. Required mutable configuration/cache/data belongs in application-appropriate user locations through an explicit adapter. Record initial creation separately from current user ownership: upgrades preserve user edits, and package removal does not delete user-modified configuration solely because pkg originally created it. A program that requires writes to its installed store tree needs a supported redirection strategy or is rejected.

## 10. Artifact identity and transformed payload integrity

Use three separate records:

1. **Artifact digest:** hash of the original `.deb`, `.rpm` or ALPM artifact, with source trust evidence. Retain the original bytes in cache according to cache policy.
2. **Derivation identity:** hash of canonical artifact identity, architecture, format-adapter/normalizer version, normalized install policy, recipe versions/parameters and all environment inputs that affect transformation output. Include the configured absolute prefix when it is embedded into files.
3. **Realization manifest:** sorted installed entries with type, normalized mode, file digest or literal symlink target, generated-file origin and transformation history. Compute a separate tree digest over this canonical manifest.

Derivation identity determines the final store path before rendering adaptations that embed that path. The output tree digest is recorded afterward; do not define the path from a hash of bytes that already contain that same path. Runtime identities hash canonical logical provider references and execution policy before materializing their own absolute paths.

Cross-package runtime references do not require rewriting another package's payload. Keep those references in runtime manifests so ordinary dependency cycles do not become circular payload identity calculations.

A transformed file is not byte-identical to the vendor payload. The original artifact's signature still authenticates the original bytes; it does not attest to pkg's transformed outputs. Immutability means no in-place changes after promotion. Reproducibility here means the same declared inputs produce the same realized manifest at the declared prefix, not reproducible upstream builds or relocation to any home directory.

Every generated configuration file, launcher and alias must appear in its owning manifest. Do not silently reuse a tree with the same derivation identity but different output digests.

## 11. Updates, removal and dependency reuse

Consider a `.deb` provider containing `libtest.so.1`, a `.rpm` consumer and an ALPM consumer:

- If both consumers' requirements and ABI evidence accept that exact provider, their runtime manifests can reference the same realization. No extra download is needed solely because their source formats differ.
- If the Debian package merely depends on `libtest.so.1`, it provides no reusable library until a concrete provider has been selected and installed.
- If the ALPM consumer needs a different symbol version, reuse fails. Select an admissible alternative for its runtime or report the unsatisfied requirement.
- Two compatible-looking names are not enough. `libtest.so.1` does not satisfy `libtestrpm.so.1` through an invented alias.

Updates build new realizations/runtime manifests and re-evaluate affected consumers. Existing runtimes retain their original provider references until a new generation deliberately changes them. Installing another provider with the same SONAME does not retarget existing consumers.

Removing an explicitly activated provider may detach its commands, but its store object remains referenced while another runtime needs it. Refuse a request that would invalidate an active dependent, or produce a separate explicit plan removing/replacing those dependents. Never leave dangling runtime references.

Keep previous generations and transaction recovery references as retention roots. The first implementation of this proposal performs no automatic physical collection of detached runtime/store objects. Safe collection while applications may still open plugins or spawn helpers requires an additional runtime-lifetime/lease design and tests. “Library already mapped” does not establish that a running program no longer needs the provider's files.

## 12. Adoption and migration

Implement within the existing `pkg-core` modules until a concrete boundary warrants another crate. Proposed responsibilities are planner/manifest construction, static host/ELF evidence, runtime adapters, transactional materialization and state/reference management.

| Slice | Deliverable | Admission gate |
|---|---|---|
| A — Contract and inspection | Complete plan types, provenance, provider evidence and truthful compatibility outcomes | G1–G2 |
| B — ELF execution | Command-specific runtime views, supported host-loader adapter and native runner | G3–G5 |
| C — Adaptations | Versioned recipes, explicit template mapping and initial interpreter adapter | G6–G7 |
| D — Publication and migration | Generation switch, durable reference graph, recovery and legacy migration | G8–G9 |
| E — Support declaration | Documented package/host corpus and command outputs | G10 |

Slices are proposal dependencies, not renamed milestones. M3 resolver evidence remains distinct from end-to-end install/runtime evidence. Generation rollback and collection intersect the existing M5 scope; desktop integration remains M4. A release implementing only part of this design must state which gates and package classes it supports.

Migration is explicit and recoverable:

1. Add versioned DB tables for manifests, provider edges and generations. Preserve legacy records as legacy/unverified; absence of new evidence is not proof of incompatibility or compatibility.
2. Inspect/replan installed packages from verified original artifacts. If bytes are unavailable, keep the old activation and report that migration requires reacquisition; do not fabricate a manifest.
3. Prepare a complete new generation before replacing a legacy profile. Preserve unmanaged or user-modified entries; if they prevent an ownership-safe cutover, report a conflict without changing that profile.
4. Freeze legacy runtime dependencies during cutover. Do not remove `profile/lib` while any retained legacy activation relies on it. Remove only entries whose ownership is established, after their consumers have migrated or been detached.
5. Commit the migration receipt and retain the previous profile representation for recovery. New executions must not mix legacy lookup paths with a supposedly verified runtime.

The existing missing-library override cannot upgrade an unresolved plan into a verified one. If retained for an experimental mode, it must persist the unresolved evidence and expose that limitation in structured output. The supported activation path described here requires complete mandatory evidence. Any resulting CLI/JSON change needs updates to the command contracts and public schema policy at implementation time.

## 13. Acceptance gates

The gate status is maintained in the [execution-gate report](../reports/2026-09-17-counter-proposal-gates.md).
The checkboxes below are closed for the declared Linux x86_64/glibc scope in
that report; they do not imply support for an absent architecture, libc,
loader, interpreter family or dynamic-plugin contract.

- [x] **G1 — Pure planning:** local/cached plans perform no writes, including at an empty data root; uncached dry-run reports missing inputs; source/state changes force revalidation; the serialized plan accounts for every realized change.
- [x] **G2 — Evidence:** malformed ELF, wrong architecture/interpreter/libc, missing providers, unsupported required symbol versions and unclassified mandatory integration fail before activation. Inspecting an artifact never runs its binaries or lifecycle scripts.
- [x] **G3 — Actual execution:** repository-owned ELF provider/consumer fixtures execute through profile commands for each supported Debian/RPM/ALPM pairing. Test real shared objects, not `.so` files containing placeholder bytes.
- [x] **G4 — Loader fidelity:** direct/transitive dependencies, differing RPATH/RUNPATH behavior, pathname dependencies, origin-relative layouts, ambient loader overrides, host-file changes and unsupported loader behavior match the declared strategy or are rejected.
- [x] **G5 — Independent commands:** two commands use different providers with the same SONAME without interference; one process requiring incompatible providers fails; installing a new package never changes an existing runtime's chosen provider. Cover helper execution and dynamic plugins within each adapter's supported contract.
- [x] **G6 — Runtime semantics:** the supported interpreter preserves arguments, uses the chosen version/module roots and validates native extensions. Unsupported versions and runtime families produce explicit outcomes. Verify the static runner bootstrap; runner tests cover ambient loader variables, spaces/metacharacters in paths and arguments, exit status, signals and generation changes.
- [x] **G7 — Adaptation integrity:** explicit `.ucf` mapping succeeds; unrelated templates are untouched; missing/ambiguous targets and copy failures abort; symlink/hardlink topology is validated; generated files are inventoried; repeated identical derivations yield identical tree digests; changed recipes/prefixes cannot silently reuse old output; mutable user configuration survives upgrade/removal.
- [x] **G8 — Transaction recovery:** inject failures before/after promotion, profile switch and DB commit. Recovery produces a consistent old or new installation without deleting reused objects or user replacements. Readers never observe a partially built generation.
- [x] **G9 — Retention and migration:** provider removal preserves referenced objects; rollback generations retain their closures; concurrent writers serialize; incomplete legacy evidence cannot be labeled verified; failed migration preserves the legacy profile and unmanaged paths.
- [x] **G10 — Support reporting:** local and repository installs expose selected providers, adaptations, omitted optional features and failure chains truthfully. Normal installation never reports success for a failed dependency. Publish a tested host/runtime/format matrix; keep unsupported combinations explicit.

These gates operationalize INV-003/004/005/006/007/008/009/010/011/012/013/014/015/018/020. Native database coexistence and system-critical rejection remain baseline INV-001/002/019 checks. Passing a parser or solver gate alone does not satisfy this execution contract.

## 14. Tradeoffs and decisions needed for acceptance

This design costs more inspection, manifest storage and compatibility-policy work. Explicit runtime selection can reject packages that the host distribution installs successfully. Host evidence checks add launch overhead; retaining objects until collection is separately proven consumes disk. These costs buy explainable provider selection and prevent unrelated installations from silently changing a command's environment.

Resolve these bounded decisions when considering acceptance:

1. **Initial supported hosts:** select the concrete glibc/architecture matrix and prove the explicit-loader strategy before enabling it. Other loaders require their own adapters; no guessed equivalence.
2. **Pure dry-run:** formalize the no-acquisition behavior in section 6, now reflected in the install implementation plan, in the canonical command contract and ADRs. Any revision to this boundary must update both documents consistently.
3. **Application libraries versus system packages:** the current [cross-distro design](cross-distro-installation.md) broadly rejects packages owning canonical `/lib` or `/usr/lib` paths. Supporting ordinary application-library providers in the isolated store requires precise canonical wording that still excludes system-critical ownership/replacement. This proposal does not silently waive the existing policy.
4. **Generation delivery scope:** coordinate publication/recovery with the M5 work. Until its gate passes, document any interim per-entry activation semantics without claiming whole-profile atomicity.

On acceptance, reconcile the [original model](rootless-execution-model.md), [store layout](store-layout.md), [filesystem/relocation policy](filesystem-layout.md), [transaction model](transaction-model.md), [compatibility model](compatibility-model.md), [database model](database-state.md), [install contract](../commands/install.md) and [install implementation plan](../../docs/install-implementation-plan.md). Record the accepted choices in the decision ledger and an ADR; update any affected existing ADR instead of treating this draft as an exception.

The governing decisions remain [ADR-010](../adr/ADR-010-profile-based-binary-activation.md), [ADR-011](../adr/ADR-011-maintainer-scripts-default-deny.md), [ADR-014](../adr/ADR-014-rootless-default.md) and [ADR-016](../adr/ADR-016-capability-based-cross-distro-compatibility.md).
