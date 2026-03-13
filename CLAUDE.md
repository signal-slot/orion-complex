# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

orion-complex is a control-plane server for an ephemeral VM lab. It manages disposable VM environments across libvirt (Linux) and Apple Virtualization.framework (macOS) providers. The server is written in Rust (edition 2024) using axum + SQLite.

## Build & Test

### Rust Server

```bash
cargo build                    # build
cargo test                     # run all tests (uses in-memory SQLite)
cargo test test_name           # run a single test
cargo clippy                   # lint
cargo fmt --check              # check formatting
```

Tests use `StubProvider` (in-process fake VM backend) and in-memory SQLite with migrations applied via `sqlx::migrate!("./migrations")`.

### Swift macOS Agent

```bash
cd macos-agent
swift build                    # build both orion-node-agent and orion-guest-agent
swift build --product orion-node-agent   # build just the node agent
swift build --product orion-guest-agent  # build just the guest agent
```

Requires macOS 13+ and Xcode with Virtualization.framework. The node agent uses `@main` via ArgumentParser — the entry point is in `NodeAgentCommand.swift` (not `main.swift`).

## Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:3000` | Server bind address |
| `DATABASE_URL` | `sqlite:orion-complex.db?mode=rwc` | SQLite connection string |
| `LIBVIRT_URI` | `qemu:///system` | libvirt hypervisor URI |
| `DATA_DIR` | `/var/lib/orion-complex` | VM data directory |
| `JWT_SECRET` | `dev-secret-change-in-production` | HMAC secret for session JWTs |
| `CORS_ORIGINS` | permissive | Comma-separated allowed origins |
| `GOOGLE_CLIENT_ID` / `MICROSOFT_CLIENT_ID` | none | OIDC provider client IDs |
| `ALLOWED_DOMAINS` | none | Comma-separated email domains for auth |

## Architecture

### Rust Server (`src/`)

- **`main.rs`** — Boots the server: config, DB pool, VM provider, background tasks (reaper + heartbeat checker), axum router with CORS/tracing.
- **`lib.rs`** — Defines `AppState` (shared across handlers): `SqlitePool`, `AuthConfig`, `reqwest::Client`, `Arc<dyn VmProvider>`.
- **`api/`** — Route handlers organized by resource. Each submodule exposes a `routes()` fn merged in `api/mod.rs`. All routes are under `/v1/`.
- **`auth.rs`** — JWT session tokens + OIDC validation (Google/Microsoft). Provides `AuthUser` and `AdminUser` axum extractors.
- **`models.rs`** — All DB models (`sqlx::FromRow`) and request types.
- **`error.rs`** — `AppError` enum implementing axum's `IntoResponse` (returns JSON `{"error": "..."}` with appropriate status codes).
- **`vm/mod.rs`** — `VmProvider` trait with methods: create, destroy, suspend, resume, reboot, snapshot, migrate. Two implementations:
  - `vm/libvirt.rs` — Production backend (libvirt/QEMU)
  - `vm/stub.rs` — Test stub (instant success, used in integration tests)
- **`background.rs`** — Background tasks: TTL reaper (destroys expired envs), heartbeat checker (marks stale nodes offline), startup reconciliation (marks stuck transient-state envs as failed).
- **`tasks.rs`** — Async task execution for VM lifecycle operations.
- **`events.rs`** — Environment event audit log.

### Scheduler

Environment placement is in `api/environments.rs`. The scheduler matches `host_os` (libvirt→linux, macos→macos) and checks resource limits (CPU/memory/disk utilization ratios, max running envs) before placing an environment on a node.

### macOS Agent (`macos-agent/`)

Swift package with two executables:
- **`orion-node-agent`** (target: `NodeAgent`) — Runs on macOS hosts, manages VMs via Apple Virtualization.framework, reports to the control plane via polling.
  - `NodeAgentCommand.swift` — Entry point, argument parsing, heartbeat loop
  - `PollCycle.swift` — Main poll loop: handles environment state transitions (creating, suspending, resuming, rebooting, migrating, destroying) and SSH key provisioning
  - `VMManager.swift` — Virtualization.framework wrapper: VM lifecycle, snapshots, migration export/import, guest provisioning via shared Virtio directory
  - `APIClient.swift` — HTTP client for control plane REST API
  - `IPSWRestore.swift` — macOS IPSW download and VM installation
  - `Config.swift` — Environment-based configuration (`ORION_CONTROL_PLANE`, `ORION_NODE_NAME`, `ORION_API_TOKEN`, `ORION_BUNDLE_STORE`, `ORION_POLL_INTERVAL`)
- **`orion-guest-agent`** (target: `GuestAgent`) — Runs inside macOS guest VMs. Watches a shared Virtio directory for provisioning commands (SSH key sync, user creation, shutdown).

### Database

SQLite with migrations in `migrations/`. Migrations run automatically on startup via sqlx. Foreign keys are enabled per-connection via PRAGMA.

### API Spec

OpenAPI 3.1 spec at `spec/openapi.yaml`.

## Key Patterns

- VM operations dispatched differently based on provider: `libvirt` provider runs in-process via `VmProvider` trait; `macos` provider is "agent-managed" — the control plane records state, and the macOS node agent polls and executes operations.
- Owner-scoped authorization: regular users only see/manage their own environments; admins see all. Implemented via `check_env_owner()` in `api/mod.rs`.
- Environment states follow a strict FSM: creating → running → suspending → suspended → resuming → running, with migration only from suspended state.
- All list endpoints support `?offset=N&limit=N` pagination (default limit 50, max 200).
