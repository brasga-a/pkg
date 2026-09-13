# Security Analysis

## Archive layer

Never trust archive entry paths. Validate before extraction and after link resolution. Enforce limits on:
- entry count;
- total expanded bytes;
- single file size;
- metadata string length;
- nesting.

## Metadata layer

Dependency expressions and scripts are attacker-controlled input. Parsers require complexity limits and clear failures.

## Repository layer

Threats:
- stale/frozen metadata;
- rollback to vulnerable package;
- mirror compromise;
- digest substitution;
- key compromise.

## Local state

Threats:
- concurrent CLI processes;
- symlink race in activation;
- transaction interruption;
- user-modified store objects.

## Runtime

pkg does not sandbox programs in MVP. Security messaging must say so explicitly.

## Privilege

Rootless install materially reduces blast radius. Any future system-wide mode should be a separate execution policy with stronger review and tests.
