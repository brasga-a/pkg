# `pkg rollback`

Selects an immutable generation retained for a profile and restores its
recorded package/activation state.

```text
pkg rollback [generation-id]
```

Without an identifier, the previous generation is selected. The command fails
closed when the generation manifest or one of its recorded store objects is
missing.
