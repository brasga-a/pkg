# Configuration

Use TOML.

Suggested rootless location:

```text
~/.config/pkg/config.toml
~/.config/pkg/repos.d/*.toml
~/.config/pkg/trust.d/*
```

## Principles

- one small general config;
- repositories modularized in `repos.d`;
- trust material separated from normal config;
- environment variables only for documented overrides;
- no executable configuration language in MVP.

Important settings:
- store/cache/state locations;
- active profile;
- repository priority;
- offline mode;
- trust policy;
- native-provider opt-in;
- telemetry/log level.
