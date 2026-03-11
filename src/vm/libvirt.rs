use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use tokio::process::Command;

use super::{VmCreateParams, VmInfo, VmProvider};

/// libvirt-based VM provider using virsh/qemu-img subprocesses.
///
/// Manages KVM/QEMU virtual machines via the `virsh` command-line tool.
/// Requires `virsh` and `qemu-img` to be available on the host.
pub struct LibvirtProvider {
    uri: String,
    data_dir: PathBuf,
}

impl LibvirtProvider {
    pub fn new(uri: &str, data_dir: &str) -> Self {
        tracing::info!("initializing libvirt provider: uri={uri}, data_dir={data_dir}");
        std::fs::create_dir_all(data_dir).ok();
        std::fs::create_dir_all(format!("{data_dir}/images")).ok();
        Self {
            uri: uri.to_string(),
            data_dir: PathBuf::from(data_dir),
        }
    }

    fn env_dir(&self, env_id: &str) -> PathBuf {
        self.data_dir.join(env_id)
    }

    fn disk_path(&self, env_id: &str) -> PathBuf {
        self.data_dir.join(env_id).join("disk.qcow2")
    }

    fn base_image_path(&self, image_name: &str) -> PathBuf {
        self.data_dir.join("images").join(format!("{image_name}.qcow2"))
    }
}

fn domain_name(provider_id: &str) -> String {
    let env_id = provider_id.strip_prefix("libvirt-").unwrap_or(provider_id);
    format!("orion-{env_id}")
}

fn env_id_from_provider_id(provider_id: &str) -> &str {
    provider_id.strip_prefix("libvirt-").unwrap_or(provider_id)
}

// ── Shell helpers ───────────────────────────────────────────────────

async fn virsh(uri: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("virsh")
        .arg("--connect")
        .arg(uri)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("failed to run virsh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "virsh {} failed: {}",
            args.join(" "),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn qemu_img(args: &[&str]) -> Result<(), String> {
    let output = Command::new("qemu-img")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("failed to run qemu-img: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("qemu-img failed: {}", stderr.trim()));
    }

    Ok(())
}

// ── XML / output parsers ────────────────────────────────────────────

fn generate_domain_xml(
    name: &str,
    vcpus: i64,
    memory_bytes: i64,
    disk_path: &Path,
    arch: &str,
) -> String {
    let memory_kib = memory_bytes / 1024;
    let arch_str = match arch {
        "arm64" | "aarch64" => "aarch64",
        _ => "x86_64",
    };

    format!(
        r#"<domain type='kvm'>
  <name>{name}</name>
  <memory unit='KiB'>{memory_kib}</memory>
  <vcpu>{vcpus}</vcpu>
  <os>
    <type arch='{arch_str}'>hvm</type>
    <boot dev='hd'/>
  </os>
  <features>
    <acpi/>
    <apic/>
  </features>
  <devices>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2' discard='unmap'/>
      <source file='{disk_path}'/>
      <target dev='vda' bus='virtio'/>
    </disk>
    <interface type='network'>
      <source network='default'/>
      <model type='virtio'/>
    </interface>
    <graphics type='vnc' port='-1' autoport='yes' listen='0.0.0.0'>
      <listen type='address' address='0.0.0.0'/>
    </graphics>
    <video>
      <model type='virtio'/>
    </video>
    <channel type='unix'>
      <target type='virtio' name='org.qemu.guest_agent.0'/>
    </channel>
    <serial type='pty'/>
    <console type='pty'/>
    <rng model='virtio'>
      <backend model='random'>/dev/urandom</backend>
    </rng>
  </devices>
</domain>"#,
        name = name,
        memory_kib = memory_kib,
        vcpus = vcpus,
        arch_str = arch_str,
        disk_path = disk_path.display(),
    )
}

fn parse_vnc_port(xml: &str) -> Option<u16> {
    for line in xml.lines() {
        let trimmed = line.trim();
        if !trimmed.contains("type='vnc'") && !trimmed.contains("type=\"vnc\"") {
            continue;
        }
        if let Some(rest) = trimmed.split("port='").nth(1)
            && let Some(val) = rest.split('\'').next()
            && let Ok(p) = val.parse::<u16>()
        {
            return Some(p);
        }
        if let Some(rest) = trimmed.split("port=\"").nth(1)
            && let Some(val) = rest.split('"').next()
            && let Ok(p) = val.parse::<u16>()
        {
            return Some(p);
        }
    }
    None
}

fn parse_ip_address(domifaddr_output: &str) -> Option<String> {
    for line in domifaddr_output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4
            && parts[2] == "ipv4"
            && let Some(addr) = parts[3].split('/').next()
            && !addr.is_empty()
        {
            return Some(addr.to_string());
        }
    }
    None
}

async fn wait_for_ip(uri: &str, domain: &str, retries: u32) -> Option<String> {
    for _ in 0..retries {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Ok(output) = virsh(uri, &["domifaddr", domain]).await
            && let Some(ip) = parse_ip_address(&output)
        {
            return Some(ip);
        }
    }
    None
}

// ── VmProvider implementation ───────────────────────────────────────

impl VmProvider for LibvirtProvider {
    fn create_vm(
        &self,
        params: VmCreateParams,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        let uri = self.uri.clone();
        let env_dir = self.env_dir(&params.env_id);
        let disk_path = self.disk_path(&params.env_id);
        let base_image = self.base_image_path(&params.image_name);

        Box::pin(async move {
            tracing::info!(
                env_id = %params.env_id,
                image = %params.image_name,
                vcpus = params.vcpus,
                memory_bytes = params.memory_bytes,
                disk_bytes = params.disk_bytes,
                "creating VM via libvirt"
            );

            // Create environment directory
            tokio::fs::create_dir_all(&env_dir)
                .await
                .map_err(|e| format!("failed to create env directory: {e}"))?;

            // Create disk image
            let disk_str = disk_path.to_string_lossy().to_string();
            let size_str = params.disk_bytes.to_string();

            if base_image.exists() {
                // COW overlay on top of base image
                let base_str = base_image.to_string_lossy().to_string();
                qemu_img(&[
                    "create",
                    "-f",
                    "qcow2",
                    "-b",
                    &base_str,
                    "-F",
                    "qcow2",
                    &disk_str,
                    &size_str,
                ])
                .await?;
                tracing::info!(env_id = %params.env_id, base = %base_str, "created COW overlay disk");
            } else {
                // Empty disk
                qemu_img(&["create", "-f", "qcow2", &disk_str, &size_str]).await?;
                tracing::info!(env_id = %params.env_id, size = %size_str, "created empty disk");
            }

            // Generate and write domain XML
            let dom_name = format!("orion-{}", params.env_id);
            let xml = generate_domain_xml(
                &dom_name,
                params.vcpus,
                params.memory_bytes,
                &disk_path,
                &params.guest_arch,
            );

            let xml_path = env_dir.join("domain.xml");
            tokio::fs::write(&xml_path, &xml)
                .await
                .map_err(|e| format!("failed to write domain XML: {e}"))?;

            // Define and start the domain
            let xml_path_str = xml_path.to_string_lossy().to_string();
            virsh(&uri, &["define", &xml_path_str]).await?;
            virsh(&uri, &["start", &dom_name]).await?;

            tracing::info!(env_id = %params.env_id, domain = %dom_name, "domain started");

            // Get VNC port from the running domain's XML
            let running_xml = virsh(&uri, &["dumpxml", &dom_name]).await.unwrap_or_default();
            let vnc_port = parse_vnc_port(&running_xml);

            // Wait for VM to obtain an IP address (up to 30s)
            let ssh_host = wait_for_ip(&uri, &dom_name, 15)
                .await
                .unwrap_or_else(|| params.node_host.clone());

            tracing::info!(
                env_id = %params.env_id,
                ssh_host = %ssh_host,
                vnc_port = ?vnc_port,
                "VM is running"
            );

            Ok(VmInfo {
                provider_id: format!("libvirt-{}", params.env_id),
                ssh_host,
                ssh_port: 22,
                vnc_host: Some(params.node_host),
                vnc_port,
            })
        })
    }

    fn destroy_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let env_dir = self.env_dir(&env_id);

        Box::pin(async move {
            tracing::info!(domain = %dom_name, "destroying VM via libvirt");

            // Force stop (ignore errors — domain might already be stopped)
            let _ = virsh(&uri, &["destroy", &dom_name]).await;

            // Undefine, removing any managed snapshots
            let _ = virsh(&uri, &["undefine", &dom_name, "--snapshots-metadata"]).await;

            // Remove disk and metadata
            if env_dir.exists() {
                tokio::fs::remove_dir_all(&env_dir)
                    .await
                    .map_err(|e| format!("failed to remove env directory: {e}"))?;
            }

            tracing::info!(domain = %dom_name, "VM destroyed");
            Ok(())
        })
    }

    fn suspend_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);

        Box::pin(async move {
            tracing::info!(domain = %dom_name, "suspending VM");
            virsh(&uri, &["suspend", &dom_name]).await?;
            Ok(())
        })
    }

    fn resume_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);

        Box::pin(async move {
            tracing::info!(domain = %dom_name, "resuming VM");
            virsh(&uri, &["resume", &dom_name]).await?;
            Ok(())
        })
    }

    fn reboot_vm(
        &self,
        provider_id: &str,
        force: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);

        Box::pin(async move {
            if force {
                tracing::info!(domain = %dom_name, "force-rebooting VM (destroy + start)");
                virsh(&uri, &["destroy", &dom_name]).await?;
                virsh(&uri, &["start", &dom_name]).await?;
            } else {
                tracing::info!(domain = %dom_name, "rebooting VM");
                virsh(&uri, &["reboot", &dom_name]).await?;
            }
            Ok(())
        })
    }

    fn get_vm_info(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let provider_id = provider_id.to_string();

        Box::pin(async move {
            // Get domain XML for VNC port
            let xml = virsh(&uri, &["dumpxml", &dom_name]).await?;
            let vnc_port = parse_vnc_port(&xml);

            // Get IP address
            let addr_output = virsh(&uri, &["domifaddr", &dom_name]).await.unwrap_or_default();
            let ssh_host = parse_ip_address(&addr_output).unwrap_or_else(|| "unknown".into());

            // Get the hypervisor hostname for VNC host
            let vnc_host = virsh(&uri, &["hostname"])
                .await
                .ok()
                .map(|s| s.trim().to_string());

            Ok(VmInfo {
                provider_id,
                ssh_host,
                ssh_port: 22,
                vnc_host,
                vnc_port,
            })
        })
    }

    fn create_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(
                domain = %dom_name,
                snapshot = %snapshot_name,
                "creating snapshot"
            );
            virsh(&uri, &["snapshot-create-as", &dom_name, &snapshot_name]).await?;
            Ok(())
        })
    }

    fn delete_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(
                domain = %dom_name,
                snapshot = %snapshot_name,
                "deleting snapshot"
            );
            virsh(&uri, &["snapshot-delete", &dom_name, &snapshot_name]).await?;
            Ok(())
        })
    }

    fn restore_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(
                domain = %dom_name,
                snapshot = %snapshot_name,
                "restoring snapshot"
            );
            virsh(&uri, &["snapshot-revert", &dom_name, &snapshot_name]).await?;
            Ok(())
        })
    }

    fn migrate_vm(
        &self,
        provider_id: &str,
        target_host: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let uri = self.uri.clone();
        let dom_name = domain_name(provider_id);
        let target_uri = format!("qemu+ssh://{target_host}/system");

        Box::pin(async move {
            tracing::info!(
                domain = %dom_name,
                target = %target_uri,
                "migrating VM"
            );
            virsh(
                &uri,
                &[
                    "migrate",
                    "--live",
                    "--persistent",
                    "--undefinesource",
                    &dom_name,
                    &target_uri,
                ],
            )
            .await?;
            tracing::info!(domain = %dom_name, "migration complete");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_vnc_port() {
        let xml = r#"
<domain type='kvm'>
  <devices>
    <graphics type='vnc' port='5901' autoport='yes' listen='0.0.0.0'>
      <listen type='address' address='0.0.0.0'/>
    </graphics>
  </devices>
</domain>"#;
        assert_eq!(parse_vnc_port(xml), Some(5901));
    }

    #[test]
    fn test_parse_vnc_port_not_found() {
        assert_eq!(parse_vnc_port("<domain/>"), None);
    }

    #[test]
    fn test_parse_ip_address() {
        let output = " Name       MAC address          Protocol     Address\n\
                       -------------------------------------------------------------------------------\n\
                        vnet0      52:54:00:ab:cd:ef    ipv4         192.168.122.45/24\n";
        assert_eq!(parse_ip_address(output), Some("192.168.122.45".into()));
    }

    #[test]
    fn test_parse_ip_address_no_match() {
        assert_eq!(parse_ip_address("no addresses"), None);
    }

    #[test]
    fn test_domain_name() {
        assert_eq!(domain_name("libvirt-abc-123"), "orion-abc-123");
        assert_eq!(domain_name("raw-id"), "orion-raw-id");
    }

    #[test]
    fn test_generate_domain_xml() {
        let xml = generate_domain_xml(
            "orion-test",
            4,
            8 * 1024 * 1024 * 1024, // 8 GB
            Path::new("/data/test/disk.qcow2"),
            "x86_64",
        );
        assert!(xml.contains("<name>orion-test</name>"));
        assert!(xml.contains("<vcpu>4</vcpu>"));
        assert!(xml.contains("<memory unit='KiB'>8388608</memory>"));
        assert!(xml.contains("/data/test/disk.qcow2"));
        assert!(xml.contains("arch='x86_64'"));
    }
}
