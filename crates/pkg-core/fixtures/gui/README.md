# GUI integration corpus

These fixtures cover the supported rootless host-integration subset:

| Fixture | Expected result |
|---|---|
| `valid.desktop` | Accepted application desktop entry |
| `valid-icon.png` | Accepted icon payload |
| `valid-mime.xml` | Accepted user MIME package description |
| `invalid.desktop` | Rejected because an application entry has no `Exec` |
| `no-integration` | Accepted package with no host-visible integration actions |

The corpus is intentionally limited to typed user-space links. It does not
exercise desktop database cache updates, services, privileged helpers or
maintainer scripts.
