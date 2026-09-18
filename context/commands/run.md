# `pkg run`

Runs a command from the active profile generation through its frozen runtime
manifest.

```text
pkg run <command> [-- <argument>...]
```

The runner clears inherited dynamic-loader overrides, installs only the
command's selected library view, preserves argument boundaries and returns the
child process status. Native ELF commands use a static `pkg-native-runner`
bootstrap with a sidecar configuration; script commands use their declared
interpreter adapter.
