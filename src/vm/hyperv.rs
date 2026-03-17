use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use tokio::process::Command;

use super::{VmCreateParams, VmInfo, VmProvider};

/// Hyper-V VM provider using PowerShell cmdlets.
///
/// Manages virtual machines via Hyper-V PowerShell commands.
/// Requires Windows with the Hyper-V role enabled and `powershell` available.
pub struct HypervProvider {
    data_dir: PathBuf,
}

impl HypervProvider {
    pub fn new(data_dir: &str) -> Self {
        tracing::info!("initializing Hyper-V provider: data_dir={data_dir}");
        std::fs::create_dir_all(data_dir).ok();
        std::fs::create_dir_all(format!("{data_dir}\\images")).ok();
        Self {
            data_dir: PathBuf::from(data_dir),
        }
    }

    fn env_dir(&self, env_id: &str) -> PathBuf {
        self.data_dir.join(env_id)
    }

    fn disk_path(&self, env_id: &str) -> PathBuf {
        self.data_dir.join(env_id).join("disk.vhdx")
    }

    fn base_image_path(&self, image_name: &str) -> PathBuf {
        self.data_dir.join("images").join(format!("{image_name}.vhdx"))
    }
}

fn vm_name(env_id: &str) -> String {
    format!("orion-{env_id}")
}

fn env_id_from_provider_id(provider_id: &str) -> &str {
    provider_id.strip_prefix("hyperv-").unwrap_or(provider_id)
}

// ── PowerShell helpers ──────────────────────────────────────────────

/// Run a PowerShell command and return stdout.
async fn powershell(script: &str) -> Result<String, String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .await
        .map_err(|e| format!("failed to run powershell: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("powershell failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a PowerShell command, ignoring output.
async fn powershell_ok(script: &str) -> Result<(), String> {
    powershell(script).await.map(|_| ())
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Get the IP address of a Hyper-V VM by querying its network adapter.
async fn get_vm_ip(vm_name: &str) -> Option<String> {
    let script = format!(
        "(Get-VMNetworkAdapter -VMName '{}' | Select-Object -ExpandProperty IPAddresses | Where-Object {{ $_ -match '^\\d+\\.\\d+\\.\\d+\\.\\d+$' }}) -join ','",
        vm_name
    );
    if let Ok(output) = powershell(&script).await {
        let ip = output.split(',').next().unwrap_or("").trim().to_string();
        if !ip.is_empty() {
            return Some(ip);
        }
    }
    None
}

/// Wait for a VM to get an IP address.
async fn wait_for_ip(vm_name: &str, retries: u32) -> Option<String> {
    for _ in 0..retries {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if let Some(ip) = get_vm_ip(vm_name).await {
            return Some(ip);
        }
    }
    None
}

/// Create a differencing VHDX on top of a parent.
async fn create_differencing_disk(child: &Path, parent: &Path) -> Result<(), String> {
    let script = format!(
        "New-VHD -Path '{}' -ParentPath '{}' -Differencing",
        child.display(),
        parent.display()
    );
    powershell_ok(&script).await
}

/// Create a new fixed/dynamic VHDX.
async fn create_disk(path: &Path, size_bytes: i64) -> Result<(), String> {
    let script = format!(
        "New-VHD -Path '{}' -SizeBytes {} -Dynamic",
        path.display(),
        size_bytes
    );
    powershell_ok(&script).await
}

/// Download or symlink an ISO file.
async fn download_iso(env_dir: &Path, url: &str) -> Result<PathBuf, String> {
    let iso_path = env_dir.join("install.iso");

    // Local file path — copy it (symlinks on Windows require elevated privileges)
    if url.starts_with('/') || url.starts_with("file://") || url.contains(":\\") {
        let src = url
            .strip_prefix("file://")
            .or_else(|| url.strip_prefix("file:"))
            .unwrap_or(url);
        if !Path::new(src).exists() {
            return Err(format!("ISO file not found: {src}"));
        }
        tokio::fs::copy(src, &iso_path)
            .await
            .map_err(|e| format!("failed to copy ISO: {e}"))?;
        return Ok(iso_path);
    }

    // Download via PowerShell (Invoke-WebRequest handles HTTPS)
    let iso_str = iso_path.to_string_lossy().to_string();
    let script = format!(
        "Invoke-WebRequest -Uri '{}' -OutFile '{}' -UseBasicParsing",
        url, iso_str
    );
    powershell(&script).await?;

    Ok(iso_path)
}

/// Create an autounattend ISO using oscdimg (Windows ADK) or mkisofs if available.
async fn create_autounattend_iso(
    env_dir: &Path,
    win_install_options_json: &str,
) -> Result<PathBuf, String> {
    let opts: super::libvirt::WinInstallOptions = serde_json::from_str(win_install_options_json)
        .map_err(|e| format!("invalid win_install_options JSON: {e}"))?;

    let unattend_dir = env_dir.join("autounattend");
    tokio::fs::create_dir_all(&unattend_dir)
        .await
        .map_err(|e| format!("failed to create autounattend dir: {e}"))?;

    let xml = super::libvirt::generate_autounattend_xml(&opts);
    let xml_path = unattend_dir.join("autounattend.xml");
    tokio::fs::write(&xml_path, &xml)
        .await
        .map_err(|e| format!("failed to write autounattend.xml: {e}"))?;

    let iso_path = env_dir.join("autounattend.iso");
    let iso_str = iso_path.to_string_lossy().to_string();
    let unattend_str = unattend_dir.to_string_lossy().to_string();

    // Try oscdimg (Windows ADK) first, fall back to mkisofs
    let result = Command::new("oscdimg")
        .args(["-l OEMDRV", "-o", &unattend_str, &iso_str])
        .output()
        .await;

    match result {
        Ok(output) if output.status.success() => return Ok(iso_path),
        _ => {
            // Fall back to mkisofs (if installed, e.g. via WSL or cygwin)
            let xml_path_str = xml_path.to_string_lossy().to_string();
            let output = Command::new("mkisofs")
                .args(["-output", &iso_str, "-volid", "OEMDRV", "-joliet", "-rock", &xml_path_str])
                .output()
                .await
                .map_err(|e| format!("neither oscdimg nor mkisofs available: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("mkisofs failed: {}", stderr.trim()));
            }
        }
    }

    Ok(iso_path)
}

// ── VmProvider implementation ───────────────────────────────────────

impl VmProvider for HypervProvider {
    fn create_vm(
        &self,
        params: VmCreateParams,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        let env_dir = self.env_dir(&params.env_id);
        let disk_path = self.disk_path(&params.env_id);
        let base_image = self.base_image_path(&params.image_name);

        Box::pin(async move {
            let is_windows = params.guest_os == "windows";
            let is_iso_install = params.iso_url.is_some();
            let name = vm_name(&params.env_id);

            tracing::info!(
                env_id = %params.env_id,
                image = %params.image_name,
                guest_os = %params.guest_os,
                vcpus = params.vcpus,
                memory_bytes = params.memory_bytes,
                disk_bytes = params.disk_bytes,
                is_iso_install = is_iso_install,
                "creating VM via Hyper-V"
            );

            // Create environment directory
            tokio::fs::create_dir_all(&env_dir)
                .await
                .map_err(|e| format!("failed to create env directory: {e}"))?;

            // Create disk image
            if base_image.exists() {
                create_differencing_disk(&disk_path, &base_image).await?;
                tracing::info!(env_id = %params.env_id, "created differencing VHDX");
            } else {
                create_disk(&disk_path, params.disk_bytes).await?;
                tracing::info!(env_id = %params.env_id, "created empty VHDX");
            }

            // Create the VM
            let memory_mb = params.memory_bytes / (1024 * 1024);
            let disk_str = disk_path.to_string_lossy().to_string();

            // Generation 2 (UEFI) for Windows 10+/Linux, Generation 1 (BIOS) for legacy
            let generation = if is_windows { 2 } else { 1 };

            let create_script = format!(
                "New-VM -Name '{name}' -MemoryStartupBytes {mem}MB -VHDPath '{disk}' -Generation {gen} -SwitchName (Get-VMSwitch | Select-Object -First 1 -ExpandProperty Name)",
                name = name,
                mem = memory_mb,
                disk = disk_str,
                gen = generation,
            );
            powershell(&create_script).await?;

            // Configure CPU count
            let cpu_script = format!(
                "Set-VMProcessor -VMName '{name}' -Count {vcpus}",
                name = name,
                vcpus = params.vcpus,
            );
            powershell_ok(&cpu_script).await?;

            // For Generation 2 VMs: disable Secure Boot for Linux guests
            if generation == 2 && !is_windows {
                let secboot_script = format!(
                    "Set-VMFirmware -VMName '{name}' -EnableSecureBoot Off",
                    name = name,
                );
                powershell_ok(&secboot_script).await?;
            }

            // Enable guest services (integration services) for IP reporting
            let integration_script = format!(
                "Enable-VMIntegrationService -VMName '{name}' -Name 'Guest Service Interface'",
                name = name,
            );
            let _ = powershell_ok(&integration_script).await;

            // Download installer ISO if provided
            if let Some(ref url) = params.iso_url {
                tracing::info!(env_id = %params.env_id, url = %url, "downloading installer ISO");
                let iso_path = download_iso(&env_dir, url).await?;
                let iso_str = iso_path.to_string_lossy().to_string();

                // Add DVD drive with installer ISO
                let dvd_script = format!(
                    "Add-VMDvdDrive -VMName '{name}' -Path '{iso}'",
                    name = name,
                    iso = iso_str,
                );
                powershell_ok(&dvd_script).await?;

                // For Gen2: set boot order — DVD first, then hard drive
                if generation == 2 {
                    let boot_script = format!(
                        "$vm = Get-VM '{name}'; \
                         $dvd = Get-VMDvdDrive -VMName '{name}'; \
                         $hd = Get-VMHardDiskDrive -VMName '{name}'; \
                         Set-VMFirmware -VMName '{name}' -BootOrder $dvd, $hd",
                        name = name,
                    );
                    powershell_ok(&boot_script).await?;
                }
            }

            // Generate autounattend ISO for Windows
            if is_windows {
                if let Some(ref json) = params.win_install_options {
                    tracing::info!(env_id = %params.env_id, "generating autounattend ISO");
                    let unattend_iso = create_autounattend_iso(&env_dir, json).await?;
                    let unattend_str = unattend_iso.to_string_lossy().to_string();
                    let dvd_script = format!(
                        "Add-VMDvdDrive -VMName '{name}' -Path '{iso}'",
                        name = name,
                        iso = unattend_str,
                    );
                    powershell_ok(&dvd_script).await?;
                }
            }

            // Enable dynamic memory (optional, improves density)
            let dynmem_script = format!(
                "Set-VMMemory -VMName '{name}' -DynamicMemoryEnabled $true -MinimumBytes {min}MB -MaximumBytes {max}MB -StartupBytes {start}MB",
                name = name,
                min = 512.min(memory_mb),
                max = memory_mb,
                start = memory_mb,
            );
            let _ = powershell_ok(&dynmem_script).await;

            // Enable checkpoints (for snapshot support)
            let checkpoint_script = format!(
                "Set-VM -VMName '{name}' -CheckpointType Standard",
                name = name,
            );
            let _ = powershell_ok(&checkpoint_script).await;

            // Start the VM
            let start_script = format!("Start-VM -Name '{name}'", name = name);
            powershell_ok(&start_script).await?;

            tracing::info!(env_id = %params.env_id, vm = %name, "VM started");

            // Wait for VM to obtain an IP address (up to 30s)
            let ssh_host = wait_for_ip(&name, 15)
                .await
                .unwrap_or_else(|| params.node_host.clone());

            tracing::info!(
                env_id = %params.env_id,
                ssh_host = %ssh_host,
                "VM is running"
            );

            Ok(VmInfo {
                provider_id: format!("hyperv-{}", params.env_id),
                ssh_host,
                ssh_port: 22,
                vnc_host: None,
                vnc_port: None,
            })
        })
    }

    fn destroy_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);
        let env_dir = self.env_dir(&env_id);

        Box::pin(async move {
            tracing::info!(vm = %name, "destroying VM via Hyper-V");

            // Force stop (ignore errors — VM might already be off)
            let _ = powershell_ok(&format!("Stop-VM -Name '{name}' -Force -TurnOff")).await;

            // Remove the VM
            let _ = powershell_ok(&format!("Remove-VM -Name '{name}' -Force")).await;

            // Remove disk and metadata
            if env_dir.exists() {
                tokio::fs::remove_dir_all(&env_dir)
                    .await
                    .map_err(|e| format!("failed to remove env directory: {e}"))?;
            }

            tracing::info!(vm = %name, "VM destroyed");
            Ok(())
        })
    }

    fn suspend_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);

        Box::pin(async move {
            tracing::info!(vm = %name, "suspending VM (saving state)");
            powershell_ok(&format!("Save-VM -Name '{name}'")).await
        })
    }

    fn resume_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);

        Box::pin(async move {
            tracing::info!(vm = %name, "resuming VM");
            powershell_ok(&format!("Start-VM -Name '{name}'")).await
        })
    }

    fn reboot_vm(
        &self,
        provider_id: &str,
        force: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);

        Box::pin(async move {
            if force {
                tracing::info!(vm = %name, "force-rebooting VM (stop + start)");
                powershell_ok(&format!("Stop-VM -Name '{name}' -Force -TurnOff")).await?;
                powershell_ok(&format!("Start-VM -Name '{name}'")).await?;
            } else {
                tracing::info!(vm = %name, "rebooting VM");
                powershell_ok(&format!("Restart-VM -Name '{name}' -Force")).await?;
            }
            Ok(())
        })
    }

    fn get_vm_info(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        let env_id = env_id_from_provider_id(&provider_id).to_string();
        let name = vm_name(&env_id);

        Box::pin(async move {
            let ssh_host = get_vm_ip(&name)
                .await
                .unwrap_or_else(|| "unknown".into());

            Ok(VmInfo {
                provider_id,
                ssh_host,
                ssh_port: 22,
                // Hyper-V uses RDP (via vmconnect) rather than VNC — no VNC port
                vnc_host: None,
                vnc_port: None,
            })
        })
    }

    fn create_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(vm = %name, snapshot = %snapshot_name, "creating checkpoint");
            powershell_ok(&format!(
                "Checkpoint-VM -Name '{name}' -SnapshotName '{snapshot_name}'"
            ))
            .await
        })
    }

    fn delete_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(vm = %name, snapshot = %snapshot_name, "removing checkpoint");
            powershell_ok(&format!(
                "Remove-VMSnapshot -VMName '{name}' -Name '{snapshot_name}'"
            ))
            .await
        })
    }

    fn restore_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);
        let snapshot_name = snapshot_name.to_string();

        Box::pin(async move {
            tracing::info!(vm = %name, snapshot = %snapshot_name, "restoring checkpoint");
            powershell_ok(&format!(
                "Restore-VMSnapshot -VMName '{name}' -Name '{snapshot_name}' -Confirm:$false"
            ))
            .await
        })
    }

    fn migrate_vm(
        &self,
        provider_id: &str,
        target_host: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let env_id = env_id_from_provider_id(provider_id).to_string();
        let name = vm_name(&env_id);
        let target_host = target_host.to_string();

        Box::pin(async move {
            tracing::info!(vm = %name, target = %target_host, "migrating VM");
            powershell_ok(&format!(
                "Move-VM -Name '{name}' -DestinationHost '{target_host}' -IncludeStorage"
            ))
            .await?;
            tracing::info!(vm = %name, "migration complete");
            Ok(())
        })
    }
}

// ── Hyper-V specific public helpers ─────────────────────────────────

/// Map an environment ID to its Hyper-V VM name.
pub fn env_vm_name(env_id: &str) -> String {
    vm_name(env_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_name() {
        assert_eq!(vm_name("abc-123"), "orion-abc-123");
    }

    #[test]
    fn test_env_id_from_provider_id() {
        assert_eq!(env_id_from_provider_id("hyperv-abc-123"), "abc-123");
        assert_eq!(env_id_from_provider_id("raw-id"), "raw-id");
    }
}
