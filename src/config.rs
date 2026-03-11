use std::env;
use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: String,
    pub libvirt_uri: String,
    pub data_dir: String,
    pub cors_origins: Option<Vec<String>>,
    pub reaper_interval_secs: u64,
    pub heartbeat_check_interval_secs: u64,
    pub heartbeat_stale_threshold_secs: i64,
}

impl Config {
    pub fn from_env() -> Self {
        let listen_addr = env::var("LISTEN_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:3000".into())
            .parse()
            .expect("invalid LISTEN_ADDR");

        let database_url =
            env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:orion-complex.db?mode=rwc".into());

        let libvirt_uri =
            env::var("LIBVIRT_URI").unwrap_or_else(|_| "qemu:///system".into());

        let data_dir =
            env::var("DATA_DIR").unwrap_or_else(|_| "/var/lib/orion-complex".into());

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

        Self {
            listen_addr,
            database_url,
            libvirt_uri,
            data_dir,
            cors_origins,
            reaper_interval_secs,
            heartbeat_check_interval_secs,
            heartbeat_stale_threshold_secs,
        }
    }
}
