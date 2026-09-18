# Agent-First Architecture & Native MCP Specification

`pkg` is designed from the ground up for a dual audience: **Human Developers** and **Autonomous AI Agents** (e.g. Google Antigravity, Claude Code, Cursor, Devin, OpenHands).

Traditional Linux package managers (`apt`, `dnf`, `pacman`) assume an interactive human operator with root (`sudo`) access. In contrast, autonomous coding agents run in restricted, headless environments where interactive prompts cause deadlocks, root access is a severe security risk, and unstructured terminal output forces brittle text scraping.

`pkg`'s rootless, isolated user-space store and capability-based resolution provide the ideal foundation for safe, deterministic agent operation.

---

## 1. The Five Contractual Pillars

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           Autonomous AI Agent                           │
└──────────────────┬──────────────────────────────────┬───────────────────┘
                   │ Tool Calling (JSON-RPC)          │ CLI Subprocess
                   ▼                                  ▼
      ┌─────────────────────────┐        ┌─────────────────────────┐
      │     Native MCP Server   │        │     Headless CLI        │
      │        `pkg mcp`        │        │   `--json` / non-tty    │
      └────────────┬────────────┘        └────────────┬────────────┘
                   │                                  │
                   └─────────────────┬────────────────┘
                                     ▼
                      ┌──────────────────────────────┐
                      │          `pkg-core`          │
                      │  Engine, Resolver & Planner  │
                      └──────────────┬───────────────┘
                                     ▼
        ┌──────────────────────────────────────────────────────────┐
        │  Isolated User-Space Store & Ephemeral Scoped Profiles   │
        └──────────────────────────────────────────────────────────┘
```

### Pillar 1: Machine-First Structured Contracts (`--json`)

Every CLI query and mutation command supports a `--json` flag. When `--json` is specified:
- Stderr and stdout contain **zero ANSI escape codes**, progress spinners, or human-oriented ASCII tables.
- Standard output contains a single, valid, schema-validated JSON document.
- In stream-oriented operations (such as multi-package batch downloads or repository syncing), `--jsonl` emits line-delimited JSON events.

#### Example: `pkg install <package> --json`
```json
{
  "status": "success",
  "package": {
    "name": "ripgrep",
    "version": "14.1.0-1",
    "format": "alpm",
    "repository": "arch-extra"
  },
  "profile": "default",
  "store_id": "4ae6b13842a5-ripgrep-14.1.0-1",
  "activated_binaries": [
    {
      "command": "rg",
      "profile_path": "/home/user/.local/share/pkg/profiles/default/bin/rg",
      "store_target": "/home/user/.local/share/pkg/store/4ae6b13842a5-ripgrep-14.1.0-1/usr/bin/rg"
    }
  ],
  "missing_host_libraries": [],
  "warnings": []
}
```

---

### Pillar 2: Non-Interactive & Fail-Safe Semantics

Under automated agent execution, **blocking on `stdin` is considered a critical system defect**.

1. **TTY Detection**: If `stdin` is not an interactive terminal (or if `--non-interactive` / `--json` / `-y` is set), `pkg` strictly disables all interactive prompts (`[y/N]`, candidate selection dialogs).
2. **Deterministic Fallbacks**:
   - Ambiguous multi-repository matches fallback automatically to the host distribution's preferred repository (via `HostFacts`), followed by configured repository priority.
   - If an ambiguity cannot be resolved automatically without human judgment, `pkg` fails immediately with exit code `22 (AMBIGUOUS_CANDIDATES)` and emits the complete candidate list in JSON for the agent to inspect and select via `--candidate <id>` or fully-qualified specifier (`repo/pkg`).
3. **Structured Diagnostics**: In failure scenarios, `--json` outputs a structured error payload detailing the failure reason, missing dependencies, or conflicting binaries.

---

### Pillar 3: Native Model Context Protocol (MCP) Server (`pkg mcp`)

`pkg` includes a built-in MCP server running over standard I/O (STDIO). AI agents connect directly to `pkg mcp` and invoke typed tools without shell command construction.

#### Core MCP Tools

| Tool | Parameters | Description |
|---|---|---|
| `pkg.install` | `target: string`, `profile?: string`, `allow_missing_libs?: bool` | Installs a package or local artifact into a target profile. |
| `pkg.remove` | `package: string`, `profile?: string` | Removes a package and its activations from a profile. |
| `pkg.search` | `query: string`, `limit?: int` | Searches available repositories for packages matching query. |
| `pkg.query_command` | `command: string` | Identifies which packages provide a specific executable binary. |
| `pkg.list` | `profile?: string` | Lists installed packages and active binaries in a profile. |
| `pkg.info` | `target: string` | Returns normalized metadata, architecture, dependencies, and payload files. |
| `pkg.profile_create` | `name: string` | Creates a new isolated profile workspace for task sandboxing. |
| `pkg.profile_drop` | `name: string` | Destroys an ephemeral profile and deactivates all its symlinks. |

#### Tool Schema Example (`pkg.install`)
```json
{
  "name": "pkg.install",
  "description": "Installs an application package safely into a user-space profile without root privileges.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "target": {
        "type": "string",
        "description": "Package name, qualified spec (e.g. 'arch-extra/discord'), local path, or URL."
      },
      "profile": {
        "type": "string",
        "description": "Target profile name (defaults to 'default')."
      },
      "allow_missing_libs": {
        "type": "boolean",
        "description": "If true, permits installation even if non-critical host shared libraries are unresolved."
      }
    },
    "required": ["target"]
  }
}
```

---

### Pillar 4: Ephemeral Agent Workspaces (Scoped Task Profiles)

Autonomous agents frequently execute transient tasks (e.g., compile a snippet, generate a document with `pandoc`, or run a linter). They must not pollute the user's permanent environment.

`pkg` provides ephemeral task profiles:

```bash
# 1. Agent provisions an isolated workspace for a sub-task:
pkg profile create agent-task-8492 --json

# 2. Agent installs required tools exclusively into the task profile:
pkg install arch-extra/weasyprint --profile agent-task-8492 --json

# 3. Agent executes the task pointing PATH to the ephemeral profile:
PATH="$HOME/.local/share/pkg/profiles/agent-task-8492/bin:$PATH" weasyprint report.html report.pdf

# 4. Agent tears down the workspace, leaving zero orphaned symlinks:
pkg profile drop agent-task-8492 --json
```

- Unreferenced store payloads can later be collected via `pkg gc`.
- If an agent task crashes or aborts, the user or agent can run `pkg profile list` to discover and drop stale agent workspaces.

---

### Pillar 5: Command and Capability-Based Discovery

Agents often know what **command** or **capability** they need to execute (e.g., `pdftotext`, `rg`, `ffmpeg`), but do not know the distribution-specific package name (e.g., `poppler-utils` on Debian vs `poppler` on Arch).

1. **`pkg query-command <command>`**:
   Queries indexed repository filelists to find packages containing `/usr/bin/<command>` or `/bin/<command>`.
2. **`pkg install --provides <command>`**:
   Resolves and installs the package that provides the requested binary.

---

## 2. Machine Exit Code Taxonomy for Agents

To enable automated error recovery without natural language parsing, `pkg` defines standardized exit codes:

| Code | Constant | Meaning | Recovery Action for Agent |
|---|---|---|---|
| `0` | `SUCCESS` | Operation completed successfully. | Proceed with task. |
| `1` | `GENERAL_ERROR` | Unclassified runtime error. | Inspect error message in JSON payload. |
| `10` | `PACKAGE_NOT_FOUND` | No package matches the requested target. | Run `pkg.search` or check spelling. |
| `20` | `CONFLICT` | Binary name collides with an already-active package. | Specify a distinct `--profile` or remove conflicting package. |
| `21` | `MISSING_DEPENDENCY` | Required dependencies or shared libraries missing. | Install missing dependency or pass `--allow-missing-libs`. |
| `22` | `AMBIGUOUS_CANDIDATES` | Multiple candidate packages match equally. | Qualify target with repository prefix (`repo/name`). |
| `30` | `INCOMPATIBLE_HOST` | Architecture or ELF ABI unsupported by host. | Select alternative package format or build. |
| `40` | `LOCK_BUSY` | Another transaction writer holds the store lock. | Wait with exponential backoff and retry. |

---

## 3. Invariants & Security Boundaries for Agents

1. **Zero Root Privilege**: `pkg` never requests or uses `sudo`/setuid. An agent running `pkg` cannot corrupt system libraries, PAM, init systems, or host package manager databases (`INV-001`, `INV-014`).
2. **Fail-Closed on Unknowns**: Unknown or unsupported lifecycle operations fail closed with typed errors rather than attempting best-effort root hooks (`INV-018`).
3. **Deterministic Cleanup**: Dropping a profile or removing a package is strictly ownership-based and completely removes all activations (`INV-014`).
