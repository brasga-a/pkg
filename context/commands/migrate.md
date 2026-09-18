# `pkg migrate`

Captures a legacy `profiles/<profile>/bin` activation as a retained generation
whose `verified` flag is false. This records what already exists without
inventing package, ABI or runtime evidence. Reacquire the original artifacts
and install them again to produce a verified generation.
