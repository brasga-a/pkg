# RPM Adapter

RPM metadata has rich Requires/Provides capability semantics and scriptlets.

Use a pure-Rust RPM parser where practical. The adapter normalizes:
- NEVRA identity;
- Requires/Provides/Conflicts/Obsoletes;
- file list and config flags;
- architecture;
- payload compression;
- scriptlets/triggers;
- signatures/digests where exposed.

## Important distinction

RPM dependencies often encode capabilities such as shared libraries, not just package names. This is closer to pkg's normalized capability model and should be preserved.

## Policy

- parse scriptlets but do not execute in MVP;
- do not open/modify the host RPM database;
- do not assume Fedora RPM policy for RPMs from arbitrary vendors;
- inspect ELF payloads after extraction and cross-check generated requirements.
