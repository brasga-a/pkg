# Store Layout

Default rootless layout should follow XDG conventions where practical:

```text
~/.local/share/pkg/
├── store/
│   └── <store-id>-<name>-<version>/
├── profiles/
│   └── default/
├── state/
├── transactions/
└── cache/

~/.local/bin/
└── pkg-managed links or a dedicated pkg bin entry
```

A safer activation layout is:

```text
~/.local/share/pkg/profiles/default/bin/
```

and the user adds that single directory to `PATH`.

## Store identity

Initial store ID:

```text
hash(artifact digest + normalized install policy + architecture)
```

Do not claim full reproducible-build identity.

## Immutability

Promoted store trees are not edited in place. Upgrade installs a new object, then activation switches.

## Side-by-side versions

```text
store/
  abc-foo-1.0/
  def-foo-2.0/
profile/default/bin/foo -> def-foo-2.0/bin/foo
```

This enables rollback without rewriting payloads.
