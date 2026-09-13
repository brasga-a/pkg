# Hooks and Maintainer Scripts

Foreign lifecycle scripts are one of the largest cross-distro hazards.

They may:
- create users/groups;
- run `ldconfig`;
- update icon/schema caches;
- manipulate systemd;
- invoke debconf;
- modify `/etc`;
- assume source-distro helper commands;
- restart services.

## MVP policy

Parse and inventory scripts. Never execute them.

A package that requires a script for correctness is classified `NeedsHostIntegration` or unsupported.

## Future model

Replace arbitrary script execution with declarative, reviewed integration capabilities:

```text
RefreshDesktopDatabase
InstallDesktopEntry
RefreshIconCache
RegisterMime
CreateUser
InstallService
```

Only low-risk reversible capabilities should become generally available.
