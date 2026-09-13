# Local Binary Management

## Activation

pkg does not copy executable payloads into a shared mutable bin directory. It creates links or generated wrappers from a profile bin directory to a selected store object.

Preferred order:
1. symlink when direct execution is correct;
2. small generated wrapper when environment/path adjustment is required;
3. reject if safe activation cannot be represented.

## Conflict policy

If two active packages provide `foo`:
- fail with an explicit conflict by default;
- allow user selection through `pkg use foo@version` or future profile semantics;
- never silently shadow an unrelated host command unless the profile PATH already intentionally precedes host paths.

## Wrappers

Wrappers may set:
- package-local library paths when safe;
- package data paths;
- application-specific environment.

Wrappers must not accept arbitrary shell fragments from metadata.

## Uninstall

Removing an inactive store object is straightforward. Removing an active object first updates activation atomically, then store reachability/GC determines deletion.
