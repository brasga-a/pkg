# Conflicts and Tensions

## C-001 — Universal UX vs distro semantics
Goal: `pkg install` everywhere.  
Constraint: packages are built under distro-specific policies.

Resolution: universal command surface, not universal semantics. Unsupported packages fail closed.

## C-002 — Native filesystem layout vs isolation
Many packages expect `/usr/...`; isolated store relocates them.

Resolution: classify relocatability; wrappers or deterministic adapters only where evidence supports them.

## C-003 — Host libraries reduce duplication vs reproducibility
Using host libraries saves disk but creates host coupling.

Resolution: MVP may satisfy verified host capabilities; do not claim reproducibility. Future bundled closures can improve isolation.

## C-004 — Rich lifecycle scripts vs safety
Scripts make packages work natively but are arbitrary mutation.

Resolution: default-deny scripts; progressively model declarative integrations.

## C-005 — One universal solver vs different version semantics
Source ecosystems use distinct ordering and expressions.

Resolution: normalized constraint IR keeps source comparators and adapters; no naive SemVer coercion.

## C-006 — Easy command exposure vs command shadowing
Putting pkg bin first in PATH can shadow host tools.

Resolution: explicit profile ordering and conflict diagnostics.
