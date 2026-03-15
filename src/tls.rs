use std::path::{Path, PathBuf};

use axum_server::tls_rustls::RustlsConfig;

use crate::config::Config;

/// Resolve TLS configuration. Returns `None` if TLS is disabled.
pub async fn resolve_tls_config(config: &Config) -> Option<RustlsConfig> {
    if !config.tls_enabled {
        return None;
    }

    let (cert_path, key_path) = match (&config.tls_cert, &config.tls_key) {
        (Some(cert), Some(key)) => (PathBuf::from(cert), PathBuf::from(key)),
        _ => {
            let dir = Path::new(&config.data_dir);
            let cert_path = dir.join("tls-cert.pem");
            let key_path = dir.join("tls-key.pem");

            if !cert_path.exists() || !key_path.exists() {
                tracing::info!("generating self-signed TLS certificate...");
                generate_self_signed(&cert_path, &key_path)
                    .expect("failed to generate self-signed certificate");
                tracing::info!("certificate written to {}", cert_path.display());
            }

            (cert_path, key_path)
        }
    };

    let tls = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .expect("failed to load TLS certificate");

    Some(tls)
}

fn generate_self_signed(
    cert_path: &Path,
    key_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = cert_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut san_names: Vec<rcgen::SanType> = vec![
        rcgen::SanType::DnsName("localhost".try_into()?),
    ];

    // Add loopback
    san_names.push(rcgen::SanType::IpAddress(
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    ));

    // Detect LAN IP
    if let Some(ip) = detect_lan_ip() {
        tracing::info!("detected LAN IP: {ip}, adding to certificate SANs");
        san_names.push(rcgen::SanType::IpAddress(ip));
    }

    let subject_names: Vec<String> = san_names
        .iter()
        .filter_map(|s| match s {
            rcgen::SanType::DnsName(n) => Some(n.as_str().to_string()),
            _ => None,
        })
        .collect();

    let mut params = rcgen::CertificateParams::new(subject_names)?;
    params.subject_alt_names = san_names;

    // Valid for 10 years
    params.not_before = rcgen::date_time_ymd(2024, 1, 1);
    params.not_after = rcgen::date_time_ymd(2034, 1, 1);

    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;

    std::fs::write(cert_path, cert.pem())?;
    std::fs::write(key_path, key_pair.serialize_pem())?;

    Ok(())
}

fn detect_lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}
