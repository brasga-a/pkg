# `pkg repo list`

List configured repositories and their current snapshot/trust status.

## Syntax

```bash
pkg repo list
```

## Suggested output

```text
ID           TYPE     PRIORITY   STATUS    LAST UPDATE   TRUST
debian-main  debian   100        ready     2h ago        trusted
vendor       debian   200        stale     9d ago        trusted
```

## Fields

- repository ID;
- adapter/type;
- configured base URL;
- priority;
- enabled/disabled;
- active snapshot identity;
- last successful refresh;
- freshness;
- trust state.

## Local only

This command does not refresh repositories.

Use:

```bash
pkg update
```

for network synchronization.
