use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(dashboard))
        .route("/api/dashboard", get(dashboard_data))
}

async fn dashboard() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        DASHBOARD_HTML,
    )
}

async fn dashboard_data(State(state): State<AppState>) -> impl IntoResponse {
    let now = crate::unix_now();

    let (total_nodes,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM nodes")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (online_nodes,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM nodes WHERE online = 1")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (running_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'running'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (creating_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'creating'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (suspended_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'suspended'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (failed_envs,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM environments WHERE state = 'failed'")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_images,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM images")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (total_users,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    let (pending_tasks,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tasks WHERE state IN ('pending', 'running')")
            .fetch_one(&state.db)
            .await
            .unwrap_or((0,));

    // Nodes list
    let nodes: Vec<crate::models::Node> =
        sqlx::query_as("SELECT * FROM nodes ORDER BY name LIMIT 20")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    let nodes_json: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            let heartbeat_ago = n
                .last_heartbeat_at
                .map(|hb| now - hb)
                .unwrap_or(-1);
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "host_os": n.host_os,
                "host_arch": n.host_arch,
                "cpu_cores": n.cpu_cores,
                "memory_gb": n.memory_bytes.map(|b| b / 1_073_741_824),
                "disk_gb": n.disk_bytes_total.map(|b| b / 1_073_741_824),
                "online": n.online == Some(1),
                "heartbeat_ago_secs": heartbeat_ago,
            })
        })
        .collect();

    // Recent environments
    let envs: Vec<crate::models::Environment> = sqlx::query_as(
        "SELECT * FROM environments ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    // For running libvirt envs, try to get VNC/SSH info
    let mut envs_json: Vec<serde_json::Value> = Vec::new();
    for e in &envs {
        let ttl_remaining = e.expires_at.map(|exp| exp - now);
        let mut entry = serde_json::json!({
            "id": e.id,
            "state": e.state,
            "provider": e.provider,
            "guest_os": e.guest_os,
            "guest_arch": e.guest_arch,
            "vcpus": e.vcpus,
            "memory_gb": e.memory_bytes.map(|b| b / 1_073_741_824),
            "disk_gb": e.disk_bytes.map(|b| b / 1_073_741_824),
            "owner_user_id": e.owner_user_id,
            "node_id": e.node_id,
            "ttl_remaining_secs": ttl_remaining,
            "created_at": e.created_at,
        });

        if e.state.as_deref() == Some("running") {
            let provider_id = format!("libvirt-{}", e.id);
            if let Ok(info) = state.vm_provider.get_vm_info(&provider_id).await {
                entry["vnc_host"] = serde_json::json!(info.vnc_host);
                entry["vnc_port"] = serde_json::json!(info.vnc_port);
                entry["ssh_host"] = serde_json::json!(info.ssh_host);
                entry["ssh_port"] = serde_json::json!(info.ssh_port);
                entry["ssh_user"] = serde_json::json!("ubuntu");
                entry["ssh_password"] = serde_json::json!("orion");
            }
        }

        envs_json.push(entry);
    }

    // Recent events
    let events: Vec<crate::models::EnvironmentEvent> = sqlx::query_as(
        "SELECT * FROM environment_events ORDER BY created_at DESC LIMIT 20",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let events_json: Vec<serde_json::Value> = events
        .iter()
        .map(|ev| {
            let ago = now - ev.created_at;
            serde_json::json!({
                "env_id": ev.env_id,
                "event_type": ev.event_type,
                "from_state": ev.from_state,
                "to_state": ev.to_state,
                "triggered_by": ev.triggered_by,
                "message": ev.message,
                "ago_secs": ago,
            })
        })
        .collect();

    axum::Json(serde_json::json!({
        "timestamp": now,
        "summary": {
            "nodes_total": total_nodes,
            "nodes_online": online_nodes,
            "environments_total": total_envs,
            "environments_running": running_envs,
            "environments_creating": creating_envs,
            "environments_suspended": suspended_envs,
            "environments_failed": failed_envs,
            "images": total_images,
            "users": total_users,
            "tasks_active": pending_tasks,
        },
        "nodes": nodes_json,
        "environments": envs_json,
        "events": events_json,
    }))
}

const DASHBOARD_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>orion-complex</title>
<style>
  :root {
    --bg: #0d1117; --surface: #161b22; --border: #30363d;
    --text: #e6edf3; --muted: #7d8590; --accent: #58a6ff;
    --green: #3fb950; --yellow: #d29922; --red: #f85149; --purple: #bc8cff;
  }
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
         background: var(--bg); color: var(--text); line-height: 1.5; }
  .container { max-width: 1200px; margin: 0 auto; padding: 24px; }
  header { display: flex; align-items: center; justify-content: space-between;
           margin-bottom: 24px; padding-bottom: 16px; border-bottom: 1px solid var(--border); }
  header h1 { font-size: 20px; font-weight: 600; }
  header h1 span { color: var(--muted); font-weight: 400; }
  .refresh-info { color: var(--muted); font-size: 13px; }
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
           gap: 12px; margin-bottom: 24px; }
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: 8px;
          padding: 16px; }
  .card .label { font-size: 12px; color: var(--muted); text-transform: uppercase;
                 letter-spacing: 0.5px; margin-bottom: 4px; }
  .card .value { font-size: 28px; font-weight: 600; }
  .card .sub { font-size: 12px; color: var(--muted); margin-top: 2px; }
  .value.green { color: var(--green); }
  .value.yellow { color: var(--yellow); }
  .value.red { color: var(--red); }
  .value.accent { color: var(--accent); }
  .value.purple { color: var(--purple); }
  section { margin-bottom: 24px; }
  section h2 { font-size: 15px; font-weight: 600; margin-bottom: 8px;
               color: var(--muted); text-transform: uppercase; letter-spacing: 0.5px; }
  table { width: 100%; border-collapse: collapse; background: var(--surface);
          border: 1px solid var(--border); border-radius: 8px; overflow: hidden;
          font-size: 13px; }
  th { text-align: left; padding: 10px 12px; background: rgba(255,255,255,0.03);
       color: var(--muted); font-weight: 500; font-size: 12px;
       text-transform: uppercase; letter-spacing: 0.5px; border-bottom: 1px solid var(--border); }
  td { padding: 8px 12px; border-bottom: 1px solid var(--border); }
  tr:last-child td { border-bottom: none; }
  .badge { display: inline-block; padding: 2px 8px; border-radius: 12px;
           font-size: 11px; font-weight: 500; }
  .badge.running { background: rgba(63,185,80,0.15); color: var(--green); }
  .badge.creating { background: rgba(88,166,255,0.15); color: var(--accent); }
  .badge.suspended { background: rgba(210,153,34,0.15); color: var(--yellow); }
  .badge.failed { background: rgba(248,81,73,0.15); color: var(--red); }
  .badge.online { background: rgba(63,185,80,0.15); color: var(--green); }
  .badge.offline { background: rgba(248,81,73,0.15); color: var(--red); }
  .badge.other { background: rgba(125,133,144,0.15); color: var(--muted); }
  .mono { font-family: 'SF Mono', Consolas, monospace; font-size: 12px; }
  .truncate { max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .empty { text-align: center; padding: 32px; color: var(--muted); }
  .arrow { color: var(--muted); margin: 0 4px; }
</style>
</head>
<body>
<div class="container">
  <header>
    <h1>orion-complex <span>/ dashboard</span></h1>
    <div class="refresh-info">auto-refreshes every 5s</div>
  </header>

  <div class="cards" id="cards"></div>

  <section>
    <h2>Nodes</h2>
    <div id="nodes-table"></div>
  </section>

  <section>
    <h2>Environments</h2>
    <div id="envs-table"></div>
  </section>

  <section>
    <h2>Recent Events</h2>
    <div id="events-table"></div>
  </section>
</div>
<script>
function badge(text) {
  const cls = ['running','creating','suspended','failed','online','offline'].includes(text) ? text : 'other';
  return `<span class="badge ${cls}">${esc(text)}</span>`;
}
function esc(s) { if (s == null) return '—'; const d = document.createElement('div'); d.textContent = String(s); return d.innerHTML; }
function shortId(s) { return s ? s.substring(0, 8) : '—'; }
function ago(secs) {
  if (secs == null || secs < 0) return '—';
  if (secs < 60) return secs + 's ago';
  if (secs < 3600) return Math.floor(secs/60) + 'm ago';
  if (secs < 86400) return Math.floor(secs/3600) + 'h ago';
  return Math.floor(secs/86400) + 'd ago';
}
function ttl(secs) {
  if (secs == null) return '<span style="color:var(--muted)">none</span>';
  if (secs <= 0) return '<span style="color:var(--red)">expired</span>';
  if (secs < 3600) return Math.floor(secs/60) + 'm';
  if (secs < 86400) return Math.floor(secs/3600) + 'h ' + Math.floor((secs%3600)/60) + 'm';
  return Math.floor(secs/86400) + 'd';
}

async function refresh() {
  try {
    const r = await fetch('/api/dashboard');
    const d = await r.json();
    const s = d.summary;

    document.getElementById('cards').innerHTML = `
      <div class="card"><div class="label">Nodes</div><div class="value green">${s.nodes_online}</div><div class="sub">${s.nodes_total} total</div></div>
      <div class="card"><div class="label">Running</div><div class="value green">${s.environments_running}</div><div class="sub">${s.environments_total} total</div></div>
      <div class="card"><div class="label">Creating</div><div class="value accent">${s.environments_creating}</div></div>
      <div class="card"><div class="label">Suspended</div><div class="value yellow">${s.environments_suspended}</div></div>
      <div class="card"><div class="label">Failed</div><div class="value red">${s.environments_failed}</div></div>
      <div class="card"><div class="label">Images</div><div class="value purple">${s.images}</div></div>
      <div class="card"><div class="label">Users</div><div class="value">${s.users}</div></div>
      <div class="card"><div class="label">Active Tasks</div><div class="value accent">${s.tasks_active}</div></div>
    `;

    if (d.nodes.length === 0) {
      document.getElementById('nodes-table').innerHTML = '<div class="empty">No nodes registered</div>';
    } else {
      document.getElementById('nodes-table').innerHTML = `<table>
        <tr><th>Name</th><th>Status</th><th>OS / Arch</th><th>CPU</th><th>Memory</th><th>Disk</th><th>Heartbeat</th></tr>
        ${d.nodes.map(n => `<tr>
          <td class="mono">${esc(n.name)}</td>
          <td>${badge(n.online ? 'online' : 'offline')}</td>
          <td>${esc(n.host_os)} / ${esc(n.host_arch)}</td>
          <td>${n.cpu_cores ?? '—'} cores</td>
          <td>${n.memory_gb != null ? n.memory_gb + ' GB' : '—'}</td>
          <td>${n.disk_gb != null ? n.disk_gb + ' GB' : '—'}</td>
          <td style="color:var(--muted)">${ago(n.heartbeat_ago_secs)}</td>
        </tr>`).join('')}
      </table>`;
    }

    if (d.environments.length === 0) {
      document.getElementById('envs-table').innerHTML = '<div class="empty">No environments</div>';
    } else {
      document.getElementById('envs-table').innerHTML = `<table>
        <tr><th>ID</th><th>State</th><th>OS / Arch</th><th>Resources</th><th>TTL</th><th>VNC</th><th>SSH</th></tr>
        ${d.environments.map(e => {
          const vnc = e.vnc_port ? `${esc(e.vnc_host ?? 'localhost')}:${e.vnc_port}` : '—';
          const ssh = e.ssh_host && e.ssh_host !== 'unknown'
            ? `ssh ${esc(e.ssh_user)}@${esc(e.ssh_host)} -p ${e.ssh_port}`
            : '—';
          const cred = e.ssh_user ? `<span style="color:var(--muted);font-size:11px">${esc(e.ssh_user)}:${esc(e.ssh_password)}</span>` : '';
          return `<tr>
          <td class="mono">${shortId(e.id)}</td>
          <td>${badge(e.state)}</td>
          <td>${esc(e.guest_os)} / ${esc(e.guest_arch)}</td>
          <td>${e.vcpus ?? '—'} vCPU, ${e.memory_gb != null ? e.memory_gb + 'G' : '—'} RAM</td>
          <td>${ttl(e.ttl_remaining_secs)}</td>
          <td class="mono" style="font-size:12px">${vnc}</td>
          <td class="mono" style="font-size:12px">${ssh}<br>${cred}</td>
        </tr>`}).join('')}
      </table>`;
    }

    if (d.events.length === 0) {
      document.getElementById('events-table').innerHTML = '<div class="empty">No events yet</div>';
    } else {
      document.getElementById('events-table').innerHTML = `<table>
        <tr><th>Time</th><th>Env</th><th>Event</th><th>Transition</th><th>By</th><th>Message</th></tr>
        ${d.events.map(ev => `<tr>
          <td style="color:var(--muted);white-space:nowrap">${ago(ev.ago_secs)}</td>
          <td class="mono">${shortId(ev.env_id)}</td>
          <td>${badge(ev.event_type)}</td>
          <td>${ev.from_state || ev.to_state ? (esc(ev.from_state) + '<span class="arrow">&rarr;</span>' + esc(ev.to_state)) : '—'}</td>
          <td>${esc(ev.triggered_by)}</td>
          <td class="truncate" style="max-width:200px">${esc(ev.message)}</td>
        </tr>`).join('')}
      </table>`;
    }
  } catch(e) {
    console.error('Dashboard refresh failed:', e);
  }
}

refresh();
setInterval(refresh, 5000);
</script>
</body>
</html>
"##;
