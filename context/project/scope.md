# Scope

## In scope

- Linux x86_64 first; aarch64 after architecture abstractions are proven.
- CLI applications and self-contained desktop applications.
- Local artifacts, then remote repositories/catalogs.
- `.deb` first, `.rpm` second, `.pkg.tar.zst` third.
- isolated package store;
- local package state;
- binary activation;
- compatibility inspection;
- checksums/signatures;
- version selection;
- deterministic uninstall;
- repository sync/cache.

## Explicitly out of initial scope

- kernel/bootloader packages;
- system libc replacement;
- init replacement;
- PAM/NSS;
- package-manager replacement for OS upgrades;
- drivers/DKMS;
- arbitrary systemd unit activation;
- SELinux policy installation;
- system users/groups from package scripts;
- automatic execution of foreign lifecycle scripts;
- source package builds;
- universal sandbox guarantee.

## Future candidates

- desktop entries/icons;
- opt-in host services;
- distro-native capability providers;
- container fallback;
- source recipes;
- pkg-native repository publishing;
- content-addressed deduplication;
- profiles/generations.
