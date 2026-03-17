use std::env;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    /// VM provider backend: "libvirt" (Linux/KVM) or "hyperv" (Windows/Hyper-V).
    pub vm_provider: String,
    pub libvirt_uri: String,
    pub data_dir: String,
    pub cors_origins: Option<Vec<String>>,
    pub reaper_interval_secs: u64,
    pub heartbeat_check_interval_secs: u64,
    pub heartbeat_stale_threshold_secs: i64,
    /// Path to TLS certificate file (PEM). If unset, auto-generates a self-signed cert.
    pub tls_cert: Option<String>,
    /// Path to TLS private key file (PEM). If unset, auto-generates a self-signed key.
    pub tls_key: Option<String>,
    /// Set to "false" or "0" to disable TLS entirely (plain HTTP).
    pub tls_enabled: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let listen_addr = env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:2743".into())
            .parse()
            .expect("invalid LISTEN_ADDR");

        let data_dir =
            env::var("DATA_DIR").unwrap_or_else(|_| "/var/lib/orion-complex".into());

        let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
            // Use DATA_DIR so the DB location is stable regardless of cwd.
            let db_path = std::path::Path::new(&data_dir).join("orion-complex.db");
            format!("sqlite:{}?mode=rwc", db_path.display())
        });

        let vm_provider =
            env::var("VM_PROVIDER").unwrap_or_else(|_| "libvirt".into());

        let libvirt_uri =
            env::var("LIBVIRT_URI").unwrap_or_else(|_| "qemu:///system".into());

        let cors_origins = env::var("CORS_ORIGINS").ok().map(|s| {
            s.split(',')
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty())
                .collect()
        });

        let reaper_interval_secs = env::var("REAPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);

        let heartbeat_check_interval_secs = env::var("HEARTBEAT_CHECK_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);

        let heartbeat_stale_threshold_secs = env::var("HEARTBEAT_STALE_THRESHOLD_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(90);

        let tls_cert = env::var("TLS_CERT").ok();
        let tls_key = env::var("TLS_KEY").ok();
        let tls_enabled = env::var("TLS_ENABLED")
            .map(|v| !matches!(v.as_str(), "false" | "0" | "no"))
            .unwrap_or(true); // TLS on by default

        Self {
            listen_addr,
            database_url,
            vm_provider,
            libvirt_uri,
            data_dir,
            cors_origins,
            reaper_interval_secs,
            heartbeat_check_interval_secs,
            heartbeat_stale_threshold_secs,
            tls_cert,
            tls_key,
            tls_enabled,
        }
    }
}
