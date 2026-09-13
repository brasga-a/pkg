# `pkg search`

Search synchronized package catalogs.

## Syntax

```bash
pkg search <query>
```

Example:

```bash
pkg search firefox
pkg search "json parser"
```

## Search domain

Search is performed over normalized repository snapshots.

Possible searchable fields:
- package name;
- summary/description;
- aliases;
- provided command/capability;
- source repository.

## Result ranking

Initial ranking should prefer:
1. exact package-name match;
2. exact provided-command match;
3. prefix name match;
4. text relevance.

Repository priority is applied after relevance rules are defined.

## Output

```text
NAME       VERSION   REPOSITORY   FORMAT   SUMMARY
firefox    ...       fedora       rpm      Web browser
```

## Offline behavior

Search works from the latest valid local snapshots.

If no snapshot exists and offline mode is active, return a clear error rather than silently querying the network.

## Security

Search results are metadata only. No package scripts or payload binaries are executed.
