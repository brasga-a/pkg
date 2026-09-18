# ALPM Python runtime fixture

Regression fixture for the scoped Python runtime launcher discovered with the Arch `reflector` package on a non-Arch host.

The package deliberately contains:

- `usr/bin/alpm-python-runtime-fixture` with an absolute `#!/usr/bin/python` shebang;
- `usr/lib/python3.14/site-packages/pkg_fixture_module.py`, imported by the executable;
- ALPM metadata declaring `depend = python`.

A correct `pkg` installation must keep the payload in the isolated store, activate a launcher in the selected profile, resolve the Python interpreter without requiring a global `/usr/bin/python`, scope `PYTHONPATH` to the package runtime, and execute the command successfully.

Expected output:

```text
MODULE_VAL=RUNTIME_LAUNCHER_SUCCESS
```

The committed package is `alpm-python-runtime-fixture-1.0.0-1-x86_64.pkg.tar.zst`.

SHA-256:

```text
76f5baf45296b97c33117e76dee90ddb860973fd30b6b99ce19d3525a7c0ede9
```

Run `./build.sh` from this directory to regenerate the archive from `src/`. The build normalizes ownership and timestamps so the fixture remains deterministic.
