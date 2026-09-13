# Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| False compatibility from dependency-name mapping | Critical | capability + ELF evidence |
| Archive traversal/symlink escape | Critical | hardened extractor + property tests |
| Foreign script mutates host | Critical | never execute by default |
| Native manager and pkg own same path | Critical | isolated store |
| Dynamic linker mismatch | High | ELF/static inspection + host facts |
| Non-relocatable package | High | classification/reject/container future |
| Repo compromise | High | source-native trust + digest verification |
| Interrupted install corrupts state | High | staging + journal + recovery |
| Solver complexity explosion | High | normalized IR, staged solver scope |
| Binary shadowing | Medium | profile conflict policy |
| Cache poisoning | High | digest-addressed promotion |
| Excess disk use | Medium | reachability-based GC later |
| Root mode expands blast radius | High | rootless default |
