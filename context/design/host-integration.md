# Host Integration Boundary

Host integration is a narrow interface rather than arbitrary package script execution.

Candidate operations:
- expose executable;
- install user desktop entry;
- install user icon;
- update user MIME metadata;
- query host capability;
- optional native dependency provider.

System-wide operations require explicit root mode and separate policy.

The host layer must know which paths/actions pkg created so uninstall can reverse them.
