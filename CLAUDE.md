# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

orion-complex is a control-plane server for an ephemeral VM lab. It manages disposable VM environments across libvirt (Linux), Hyper-V (Windows), and Apple Virtualization.framework + QEMU (macOS) providers. Three components work together: a Rust server, a Next.js web frontend, and a Swift macOS node agent.

## Build & Test

### Rust Server

```bash
cargo build                    # build
cargo test                     # run all tests (uses in-memory SQLite)
cargo test test_name           # run a single test
cargo clippy                   # lint
cargo fmt --check              # check formatting
```

Tests use `StubProvider` (in-process fake VM backend) and in-memory SQLite with migrations applied via `sqlx::migrate!("./migrations")`. Integration tests are in `tests/api_tests.rs` — they build a full `axum::Router` with `AppState`, seed test users, and use `tower::ServiceExt::oneshot()` to send requests (no live server needed). Rust edition is 2024.

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

Requires macOS 13+ and Xcode with Virtualization.framework. The node agent uses `@main` via ArgumentParser — the entry point is in `NodeAgentCommand.swift` (not `main.swift`). After building, the binary must be signed with the virtualization entitlement:

```bash
codesign --force --sign - --entitlements entitlements.plist .build/debug/orion-node-agent
# entitlements.plist must contain com.apple.security.virtualization = true
```

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
| `DATABASE_URL` | `sqlite:{DATA_DIR}/orion-complex.db?mode=rwc` | SQLite connection string |
| `VM_PROVIDER` | `libvirt` | VM backend: `libvirt` (Linux/KVM) or `hyperv` (Windows/Hyper-V) |
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
- **`lib.rs`** — `AppState` shared across handlers: `SqlitePool`, `AuthConfig`, `reqwest::Client`, `Arc<dyn VmProvider>`. Also has `delete_environment_cascade()` for cascading deletes.
- **`config.rs`** — `Config::from_env()` reads all env vars with defaults.
- **`db.rs`** — Creates SQLite pool, enables foreign keys per-connection, runs migrations.
- **`models.rs`** — All DB row structs (`#[derive(sqlx::FromRow)]`) and API request types. SQLite booleans are `i64` (0/1).
- **`error.rs`** — `AppError` enum → axum `IntoResponse` (returns `{"error": "..."}` JSON). `sqlx::Error::RowNotFound` auto-maps to 404.
- **`events.rs`** / **`tasks.rs`** — Helpers for recording environment events and async task tracking.
- **`api/`** — Route handlers by resource, each exposes `routes()` merged in `api/mod.rs`. All under `/v1/`. Shared `PaginationParams` and `fetch_env()` in `api/mod.rs`.
- **`auth.rs`** — JWT session tokens + OIDC. `AuthUser` and `AdminUser` axum extractors.
- **`api/webauthn.rs`** — TOTP (code-only login, server iterates all users) and WebAuthn passkey auth.
- **`api/ws.rs`** — WebSocket proxy for VNC and SSH. VNC: raw TCP relay (WebSocket ↔ `/usr/bin/nc` ↔ VM VNC port). SSH: waits for user credentials from the browser as JSON `{username, password, cols, rows}`, then authenticates via `russh` and relays PTY I/O. Both use `/usr/bin/nc` to bypass macOS 15+ Local Network Privacy.
- **`api/uploads.rs`** — Multipart file upload handling for VM images.
- **`tls.rs`** — Self-signed certificate generation via `rcgen` with localhost + LAN IP SANs.
- **`vm/mod.rs`** — `VmProvider` trait + `provider_id_for()` helper. `vm/libvirt.rs` (Linux/KVM), `vm/hyperv.rs` (Windows/Hyper-V), `vm/stub.rs` (tests).
- **`background.rs`** — TTL reaper, heartbeat checker, startup reconciliation.

### macOS Agent (`macos-agent/`)

- **`NodeAgent`** — Polls control plane, executes VM lifecycle transitions.
  - `VMManager.swift` — Virtualization.framework for arm64 macOS VMs; QEMU TCG for x86_64 Linux VMs.
  - `PollCycle.swift` — State machine: creating → running, suspending, resuming, destroying. Reports VM internal IP endpoints when port forwarding is off (for web proxy), or host LAN IP + forwarded ports when on.
  - `PortForwarder.swift` — TCP port forwarding (host LAN → VM private network) for external client access. Swift TCP listener on `0.0.0.0` + `/usr/bin/nc` per-connection bridge to bypass macOS Local Network Privacy. Controlled by `port_forwarding` flag on environment.
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
- **VNC/SSH proxy**: Backend uses `/usr/bin/nc` to connect to VMs (bypasses macOS Local Network Privacy). When port forwarding is off, connects to VM internal IP (`192.168.64.x`). When on, endpoints point to host LAN IP + forwarded ports. Browser noVNC works for non-macOS VMs only — macOS Screen Sharing uses proprietary Apple DH auth that noVNC doesn't support; use native `vnc://` client instead.
- **SSH credentials**: Never auto-try passwords. The web SSH terminal prompts users for username and password, which are sent to the backend via the first WebSocket message.
- **Windows VM support**: Both libvirt (Linux/KVM) and macOS agent (QEMU) providers support Windows guest VMs from ISO with autounattend.xml for unattended install. The `win_install_options` JSON field stores install options (hardware bypass, language, user account, auto-partition, etc.). The libvirt provider uses SATA disk bus during ISO install (no virtio drivers in Windows installer), Hyper-V enlightenments, QXL video, and USB tablet input. The `provider` field in `CreateEnvironmentRequest` controls which backend handles ISO installs (default: `"virtualization"`; use `"libvirt"` for KVM).
- **Environment FSM**: creating → running → suspending → suspended → resuming → running. Migration only from suspended state.
- **Auth**: TOTP is the default login method. Code-only login — server iterates all users with `totp_secret` set to find match. WebAuthn passkeys also supported.
- **noVNC bundling**: npm package ships broken CJS with top-level `await`. Build script patches it before rollup bundles into browser IIFE. See `web/rollup.novnc.mjs` and `build:novnc` script in `package.json`.
- **All list endpoints**: `?offset=N&limit=N` pagination (default 50, max 200).
- **All users see all environments** — this is a shared-resource system, not multi-tenant.

## Database

SQLite with migrations in `migrations/`. Migrations run automatically on startup. Foreign keys enabled per-connection via PRAGMA. API spec at `spec/openapi.yaml`.
