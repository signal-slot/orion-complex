# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

orion-complex is a control-plane server for an ephemeral VM lab. It manages disposable VM environments across libvirt (Linux) and Apple Virtualization.framework + QEMU (macOS) providers. Three components work together: a Rust server, a Next.js web frontend, and a Swift macOS node agent.

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

### Web Frontend

```bash
cd web
npm install                    # install deps + auto-builds noVNC bundle (postinstall)
npm run dev                    # starts custom HTTPS server on port 2742
npm run build                  # production build (builds noVNC + next build)
npm run build:novnc            # rebuild noVNC bundle only
```

Next.js 16+ with React 19, Tailwind CSS 4, TypeScript. Uses App Router with `(authenticated)` route group for protected pages.

### Swift macOS Agent

```bash
cd macos-agent
swift build                    # build both orion-node-agent and orion-guest-agent
swift build --product orion-node-agent   # build just the node agent
swift build --product orion-guest-agent  # build just the guest agent
```

Requires macOS 13+ and Xcode with Virtualization.framework. The node agent uses `@main` via ArgumentParser — the entry point is in `NodeAgentCommand.swift` (not `main.swift`).

## Running in Development

Three processes are needed. Backend runs HTTP (no TLS) on localhost; frontend terminates HTTPS and proxies API requests.

```bash
# 1. Rust backend (port 2743, HTTP only in dev)
TLS_ENABLED=false cargo run

# 2. Next.js frontend (port 2742, HTTPS with self-signed cert)
cd web && npm run dev

# 3. macOS node agent (optional, needed for VZ/QEMU VM management)
cd macos-agent && ORION_CONTROL_PLANE=http://127.0.0.1:2743 \
  ORION_API_TOKEN=<jwt> ORION_NODE_ID=<uuid> .build/debug/orion-node-agent
```

Port 2742 is the frontend port. Port 2743 is the backend port. Never use port 3000.

## Environment Variables

| Variable | Default | Purpose |
|---|---|---|
| `LISTEN_ADDR` | `127.0.0.1:2743` | Server bind address |
| `DATABASE_URL` | `sqlite:orion-complex.db?mode=rwc` | SQLite connection string |
| `LIBVIRT_URI` | `qemu:///system` | libvirt hypervisor URI |
| `DATA_DIR` | `/var/lib/orion-complex` | VM data directory |
| `JWT_SECRET` | `dev-secret-change-in-production` | HMAC secret for session JWTs |
| `CORS_ORIGINS` | permissive | Comma-separated allowed origins |
| `TLS_ENABLED` | `true` | Set to `false` to disable TLS (plain HTTP) |
| `TLS_CERT` / `TLS_KEY` | auto-generated | Path to TLS PEM files |

Node agent env vars: `ORION_CONTROL_PLANE`, `ORION_NODE_NAME`, `ORION_NODE_ID`, `ORION_API_TOKEN`, `ORION_BUNDLE_STORE`, `ORION_POLL_INTERVAL`.

## Architecture

### Three-Component System

```
Browser → HTTPS (port 2742) → Next.js custom server (server.mjs)
                                 ├── /v1/* → HTTP proxy → Rust backend (port 2743)
                                 ├── /v1/*/ws/* → WebSocket proxy → Rust backend
                                 └── everything else → Next.js App Router

Rust backend ← polls ← macOS node agent (Swift)
             → TCP proxy → VNC/SSH on VMs
```

The custom server (`web/server.mjs`) exists because Next.js rewrites cannot proxy WebSocket connections. It uses `http-proxy` to forward both HTTP and WebSocket `/v1/*` requests to the backend, while serving the frontend on the same HTTPS port.

### Rust Server (`src/`)

- **`main.rs`** — Boots server: config, DB pool, VM provider, background tasks, axum router.
- **`lib.rs`** — `AppState` shared across handlers: `SqlitePool`, `AuthConfig`, `reqwest::Client`, `Arc<dyn VmProvider>`.
- **`api/`** — Route handlers by resource, each exposes `routes()` merged in `api/mod.rs`. All under `/v1/`.
- **`auth.rs`** — JWT session tokens + OIDC. `AuthUser` and `AdminUser` axum extractors.
- **`api/webauthn.rs`** — TOTP (code-only login, server iterates all users) and WebAuthn passkey auth.
- **`api/ws.rs`** — WebSocket proxy for VNC and SSH. Resolves target by looking up environment's `vnc_host`/`vnc_port` or `ssh_host`/`ssh_port`, then bridges WebSocket ↔ TCP.
- **`tls.rs`** — Self-signed certificate generation via `rcgen` with localhost + LAN IP SANs.
- **`vm/mod.rs`** — `VmProvider` trait. `vm/libvirt.rs` (production), `vm/stub.rs` (tests).
- **`background.rs`** — TTL reaper, heartbeat checker, startup reconciliation.

### macOS Agent (`macos-agent/`)

- **`NodeAgent`** — Polls control plane, executes VM lifecycle transitions.
  - `VMManager.swift` — Virtualization.framework for arm64 macOS VMs; QEMU TCG for x86_64 Linux VMs.
  - `PollCycle.swift` — State machine: creating → running, suspending, resuming, destroying. Reports VNC/SSH endpoints directly (VM internal IP, no port forwarding needed for web proxy).
  - `PortForwarder.swift` — Optional port forwarding for external VNC/SSH client access.
- **`GuestAgent`** — Runs inside macOS guest VMs, watches shared Virtio directory for provisioning.

### Web Frontend (`web/`)

- App Router with `(authenticated)` layout group wrapping protected routes.
- `lib/auth.ts` — Token storage in localStorage, `AuthProvider` context.
- `lib/api.ts` — API client, all requests go to same origin (proxied by server.mjs).
- `components/Sidebar.tsx` — Responsive sidebar, mobile drawer.
- noVNC loaded as pre-built IIFE bundle from `public/novnc-rfb.js`.
- xterm.js for SSH terminal.

## Key Patterns

- **Agent-managed providers**: `is_agent_managed()` returns true for `"macos"` and `"virtualization"` providers. These don't use the `VmProvider` trait — the control plane records desired state, and the node agent polls and executes.
- **VNC/SSH proxy**: Backend connects directly to VM internal IPs (VZ: `192.168.64.x:5900`, QEMU: `127.0.0.1:VNC_PORT`). No port forwarding needed for web access.
- **Environment FSM**: creating → running → suspending → suspended → resuming → running. Migration only from suspended state.
- **Auth**: TOTP is the default login method. Code-only login — server iterates all users with `totp_secret` set to find match. WebAuthn passkeys also supported.
- **noVNC bundling**: npm package ships broken CJS with top-level `await`. Build script patches it before rollup bundles into browser IIFE. See `web/rollup.novnc.mjs` and `build:novnc` script in `package.json`.
- **All list endpoints**: `?offset=N&limit=N` pagination (default 50, max 200).
- **All users see all environments** — this is a shared-resource system, not multi-tenant.

## Database

SQLite with migrations in `migrations/`. Migrations run automatically on startup. Foreign keys enabled per-connection via PRAGMA. API spec at `spec/openapi.yaml`.
