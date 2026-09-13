# MVP

## Product claim

> pkg can safely install a supported local application package into its own user-space store and expose its commands without handing file ownership to the host package manager.

## Supported MVP

- Linux x86_64;
- local `.deb`;
- package classes that do not require maintainer scripts;
- store/profile/state;
- ELF host-library checks;
- checksum computed locally;
- install/remove/list/info/dry-run.

## Not claimed

- all `.deb` files work;
- remote Debian repositories;
- RPM/Arch packages;
- system services;
- distro upgrades;
- universal dependency solving.
