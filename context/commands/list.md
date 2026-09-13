# `pkg list`

List locally installed packages.

## Syntax

```bash
pkg list
```

Potential filters:

```bash
pkg list --profile default
pkg list --outdated
pkg list --format deb
```

## Default output

Suggested fields:

```text
NAME        VERSION   STATUS    SOURCE       ACTIVE
ripgrep     14.1.0    installed debian-main yes
foo         2.0.0     installed local        no
```

## Meaning of installed

A package is installed when pkg state references a valid store object.

`ACTIVE` means the selected profile exposes that package/version.

This distinction matters because side-by-side versions or rollback generations may retain inactive objects.

## Data source

`pkg list` reads the local state database. It should not scan arbitrary filesystem directories as its primary authority.

If state and filesystem disagree, display a consistency warning and recommend:

```bash
pkg doctor
```

## No network requirement

`pkg list` is purely local and should work offline.
