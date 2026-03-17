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

/// Options controlling domain XML generation.
struct DomainXmlOptions<'a> {
    name: &'a str,
    vcpus: i64,
    memory_bytes: i64,
    disk_path: &'a Path,
    /// cloud-init seed ISO (Linux)
    seed_iso_path: Option<&'a Path>,
    arch: &'a str,
    /// true for Windows guest VMs
    is_windows: bool,
    /// Windows installer ISO
    install_iso_path: Option<&'a Path>,
    /// autounattend ISO for Windows unattended install
    autounattend_iso_path: Option<&'a Path>,
}

fn generate_domain_xml(opts: &DomainXmlOptions) -> String {
    let memory_kib = opts.memory_bytes / 1024;
    let arch_str = match opts.arch {
        "arm64" | "aarch64" => "aarch64",
        _ => "x86_64",
    };

    // --- CDROM devices ---
    let mut cdrom_xml = String::new();

    // For Windows ISO install: attach the installer ISO
    if let Some(iso) = opts.install_iso_path {
        cdrom_xml += &format!(
            r#"
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{}'/>
      <target dev='hda' bus='sata'/>
      <readonly/>
    </disk>"#,
            iso.display()
        );
    }

    // Autounattend ISO for Windows
    if let Some(iso) = opts.autounattend_iso_path {
        cdrom_xml += &format!(
            r#"
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{}'/>
      <target dev='hdb' bus='sata'/>
      <readonly/>
    </disk>"#,
            iso.display()
        );
    }

    // Cloud-init seed ISO (Linux)
    if let Some(p) = opts.seed_iso_path {
        cdrom_xml += &format!(
            r#"
    <disk type='file' device='cdrom'>
      <driver name='qemu' type='raw'/>
      <source file='{}'/>
      <target dev='hdc' bus='scsi'/>
      <readonly/>
    </disk>
    <controller type='scsi' model='virtio-scsi'/>"#,
            p.display()
        );
    }

    // --- Disk bus: Windows needs SATA during install (no virtio drivers), Linux uses virtio ---
    let (disk_dev, disk_bus) = if opts.is_windows && opts.install_iso_path.is_some() {
        ("sda", "sata")
    } else {
        ("vda", "virtio")
    };

    // Use OVMF for UEFI boot if available
    let ovmf_paths = [
        "/usr/share/edk2-ovmf/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    ];
    let ovmf_code = ovmf_paths.iter().find(|p| std::path::Path::new(p).exists());

    // SMBIOS hint for cloud-init NoCloud datasource detection
    let has_cloud_init = opts.seed_iso_path.is_some();
    let qemu_ns_xml = if has_cloud_init {
        r#"
  <qemu:commandline>
    <qemu:arg value='-smbios'/>
    <qemu:arg value='type=1,serial=ds=nocloud'/>
  </qemu:commandline>"#
    } else {
        ""
    };

    let domain_attrs = if has_cloud_init {
        "type='kvm' xmlns:qemu='http://libvirt.org/schemas/domain/qemu/1.0'"
    } else {
        "type='kvm'"
    };

    // Boot order: cdrom first for ISO installs, then disk
    let boot_xml = if opts.install_iso_path.is_some() {
        format!(
            r#"<boot dev='cdrom'/>
    <boot dev='hd'/>"#
        )
    } else {
        "<boot dev='hd'/>".to_string()
    };

    let os_xml = if let Some(fw) = ovmf_code {
        format!(
            r#"<os>
    <type arch='{arch_str}' machine='q35'>hvm</type>
    <loader readonly='yes' type='pflash'>{fw}</loader>
    {boot_xml}
  </os>"#
        )
    } else {
        format!(
            r#"<os>
    <type arch='{arch_str}'>hvm</type>
    {boot_xml}
  </os>"#
        )
    };

    // Windows-specific features: Hyper-V enlightenments for better performance
    let features_xml = if opts.is_windows {
        r#"<features>
    <acpi/>
    <apic/>
    <hyperv mode='custom'>
      <relaxed state='on'/>
      <vapic state='on'/>
      <spinlocks state='on' retries='8191'/>
    </hyperv>
  </features>"#
    } else {
        r#"<features>
    <acpi/>
    <apic/>
  </features>"#
    };

    // Windows uses QXL video for better display support; Linux uses virtio
    let video_xml = if opts.is_windows {
        r#"<video>
      <model type='qxl' ram='65536' vram='65536'/>
    </video>"#
    } else {
        r#"<video>
      <model type='virtio'/>
    </video>"#
    };

    // Windows needs a USB tablet for proper mouse input
    let input_xml = if opts.is_windows {
        r#"
    <input type='tablet' bus='usb'/>
    <controller type='usb' model='qemu-xhci'/>"#
    } else {
        ""
    };

    format!(
        r#"<domain {domain_attrs}>
  <name>{name}</name>
  <memory unit='KiB'>{memory_kib}</memory>
  <vcpu>{vcpus}</vcpu>
  {os_xml}
  {features_xml}
  <devices>
    <disk type='file' device='disk'>
      <driver name='qemu' type='qcow2' discard='unmap'/>
      <source file='{disk_path}'/>
      <target dev='{disk_dev}' bus='{disk_bus}'/>
    </disk>{cdrom_xml}
    <interface type='network'>
      <source network='default'/>
      <model type='virtio'/>
    </interface>
    <graphics type='vnc' port='-1' autoport='yes' listen='0.0.0.0'>
      <listen type='address' address='0.0.0.0'/>
    </graphics>
    {video_xml}{input_xml}
    <channel type='unix'>
      <target type='virtio' name='org.qemu.guest_agent.0'/>
    </channel>
    <serial type='pty'/>
    <console type='pty'/>
    <rng model='virtio'>
      <backend model='random'>/dev/urandom</backend>
    </rng>
  </devices>{qemu_ns_xml}
</domain>"#,
        domain_attrs = domain_attrs,
        name = opts.name,
        memory_kib = memory_kib,
        vcpus = opts.vcpus,
        os_xml = os_xml,
        features_xml = features_xml,
        disk_path = opts.disk_path.display(),
        disk_dev = disk_dev,
        disk_bus = disk_bus,
        cdrom_xml = cdrom_xml,
        video_xml = video_xml,
        input_xml = input_xml,
        qemu_ns_xml = qemu_ns_xml,
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

async fn create_cloud_init_seed(
    env_dir: &Path,
    env_id: &str,
    ssh_authorized_keys: &[String],
) -> Result<PathBuf, String> {
    let seed_dir = env_dir.join("seed");
    tokio::fs::create_dir_all(&seed_dir)
        .await
        .map_err(|e| format!("failed to create seed dir: {e}"))?;

    let meta_data = format!(
        "instance-id: {env_id}\nlocal-hostname: orion\n"
    );

    let ssh_keys_yaml = if ssh_authorized_keys.is_empty() {
        String::new()
    } else {
        let keys: Vec<String> = ssh_authorized_keys
            .iter()
            .map(|k| format!("  - {k}"))
            .collect();
        format!("ssh_authorized_keys:\n{}\n", keys.join("\n"))
    };

    let user_data = format!(
        r#"#cloud-config
password: orion
chpasswd:
  expire: false
ssh_pwauth: true
{ssh_keys_yaml}"#
    );

    // NoCloud network-config v2: enable DHCP on all ethernets
    let network_config = r#"version: 2
ethernets:
  nics:
    match:
      name: "e*"
    dhcp4: true
"#;

    tokio::fs::write(seed_dir.join("meta-data"), &meta_data)
        .await
        .map_err(|e| format!("failed to write meta-data: {e}"))?;
    tokio::fs::write(seed_dir.join("user-data"), &user_data)
        .await
        .map_err(|e| format!("failed to write user-data: {e}"))?;
    tokio::fs::write(seed_dir.join("network-config"), network_config)
        .await
        .map_err(|e| format!("failed to write network-config: {e}"))?;

    let iso_path = env_dir.join("seed.iso");
    let iso_str = iso_path.to_string_lossy().to_string();
    let seed_str = seed_dir.to_string_lossy().to_string();

    let output = Command::new("mkisofs")
        .args([
            "-output", &iso_str,
            "-volid", "cidata",
            "-joliet",
            "-rock",
            &format!("{}/meta-data", seed_str),
            &format!("{}/user-data", seed_str),
            &format!("{}/network-config", seed_str),
        ])
        .output()
        .await
        .map_err(|e| format!("failed to run mkisofs: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("mkisofs failed: {}", stderr.trim()));
    }

    Ok(iso_path)
}

// ── Windows autounattend support ────────────────────────────────────

/// Windows install options, deserialized from JSON stored in the DB.
#[derive(Debug, Clone, serde::Deserialize)]
struct WinInstallOptions {
    bypass_tpm: Option<bool>,
    bypass_secure_boot: Option<bool>,
    bypass_ram: Option<bool>,
    bypass_cpu: Option<bool>,
    language: Option<String>,
    timezone: Option<String>,
    username: Option<String>,
    password: Option<String>,
    auto_login: Option<bool>,
    auto_partition: Option<bool>,
    product_key: Option<String>,
    skip_oobe: Option<bool>,
}

fn generate_autounattend_xml(opts: &WinInstallOptions) -> String {
    let arch = "amd64";
    let token = "31bf3856ad364e35";
    let lang = opts.language.as_deref().unwrap_or("en-US");

    // -- windowsPE pass --
    let mut run_sync_cmds = String::new();
    let mut order = 1u32;
    let bypasses = [
        (opts.bypass_tpm, "BypassTPMCheck"),
        (opts.bypass_secure_boot, "BypassSecureBootCheck"),
        (opts.bypass_ram, "BypassRAMCheck"),
        (opts.bypass_cpu, "BypassCPUCheck"),
    ];
    for (flag, name) in &bypasses {
        if *flag == Some(true) {
            run_sync_cmds += &format!(
                r#"        <RunSynchronousCommand wcm:action="add">
          <Order>{order}</Order>
          <Path>reg add HKLM\SYSTEM\Setup\LabConfig /v {name} /t REG_DWORD /d 1 /f</Path>
        </RunSynchronousCommand>
"#
            );
            order += 1;
        }
    }

    let mut setup_component = String::new();
    if !run_sync_cmds.is_empty() {
        setup_component += &format!("      <RunSynchronous>\n{run_sync_cmds}      </RunSynchronous>\n");
    }
    if opts.auto_partition == Some(true) {
        setup_component += r#"      <DiskConfiguration>
        <Disk wcm:action="add">
          <DiskID>0</DiskID>
          <WillWipeDisk>true</WillWipeDisk>
          <CreatePartitions>
            <CreatePartition wcm:action="add">
              <Order>1</Order>
              <Type>Primary</Type>
            </CreatePartition>
          </CreatePartitions>
          <ModifyPartitions>
            <ModifyPartition wcm:action="add">
              <Order>1</Order>
              <PartitionID>1</PartitionID>
              <Format>NTFS</Format>
              <Label>Windows</Label>
              <Letter>C</Letter>
            </ModifyPartition>
          </ModifyPartitions>
        </Disk>
      </DiskConfiguration>
      <ImageInstall>
        <OSImage>
          <InstallTo>
            <DiskID>0</DiskID>
            <PartitionID>1</PartitionID>
          </InstallTo>
        </OSImage>
      </ImageInstall>
"#;
    }
    if let Some(ref key) = opts.product_key {
        if !key.is_empty() {
            setup_component += &format!(
                r#"      <UserData>
        <ProductKey>
          <Key>{key}</Key>
        </ProductKey>
        <AcceptEula>true</AcceptEula>
      </UserData>
"#
            );
        }
    }

    let mut windows_pe = String::new();
    if !setup_component.is_empty() {
        windows_pe += &format!(
            r#"    <component name="Microsoft-Windows-Setup"
               processorArchitecture="{arch}"
               publicKeyToken="{token}"
               language="neutral"
               versionScope="nonSxS">
{setup_component}    </component>
"#
        );
    }
    // Language for WinPE
    windows_pe += &format!(
        r#"    <component name="Microsoft-Windows-International-Core-WinPE"
               processorArchitecture="{arch}"
               publicKeyToken="{token}"
               language="neutral"
               versionScope="nonSxS">
      <SetupUILanguage>
        <UILanguage>{lang}</UILanguage>
      </SetupUILanguage>
      <InputLocale>{lang}</InputLocale>
      <SystemLocale>{lang}</SystemLocale>
      <UILanguage>{lang}</UILanguage>
      <UserLocale>{lang}</UserLocale>
    </component>
"#
    );

    // -- specialize pass --
    let mut specialize = String::new();
    if let Some(ref tz) = opts.timezone {
        if !tz.is_empty() {
            specialize = format!(
                r#"    <component name="Microsoft-Windows-Shell-Setup"
               processorArchitecture="{arch}"
               publicKeyToken="{token}"
               language="neutral"
               versionScope="nonSxS">
      <TimeZone>{tz}</TimeZone>
    </component>
"#
            );
        }
    }

    // -- oobeSystem pass --
    let mut oobe_inner = String::new();
    if opts.skip_oobe == Some(true) {
        oobe_inner += r#"      <OOBE>
        <HideEULAPage>true</HideEULAPage>
        <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>
        <ProtectYourPC>3</ProtectYourPC>
        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>
        <SkipMachineOOBE>true</SkipMachineOOBE>
        <SkipUserOOBE>true</SkipUserOOBE>
      </OOBE>
"#;
    }
    if let Some(ref username) = opts.username {
        if !username.is_empty() {
            let password = opts.password.as_deref().unwrap_or("");
            oobe_inner += &format!(
                r#"      <UserAccounts>
        <LocalAccounts>
          <LocalAccount wcm:action="add">
            <Name>{username}</Name>
            <Group>Administrators</Group>
            <Password>
              <Value>{password}</Value>
              <PlainText>true</PlainText>
            </Password>
          </LocalAccount>
        </LocalAccounts>
      </UserAccounts>
"#
            );
            if opts.auto_login == Some(true) {
                oobe_inner += &format!(
                    r#"      <AutoLogon>
        <Enabled>true</Enabled>
        <Username>{username}</Username>
        <Password>
          <Value>{password}</Value>
          <PlainText>true</PlainText>
        </Password>
        <LogonCount>1</LogonCount>
      </AutoLogon>
"#
                );
            }
        }
    }

    let mut oobe_system = String::new();
    if !oobe_inner.is_empty() {
        oobe_system += &format!(
            r#"    <component name="Microsoft-Windows-Shell-Setup"
               processorArchitecture="{arch}"
               publicKeyToken="{token}"
               language="neutral"
               versionScope="nonSxS">
{oobe_inner}    </component>
"#
        );
    }
    // Language in oobeSystem
    oobe_system += &format!(
        r#"    <component name="Microsoft-Windows-International-Core"
               processorArchitecture="{arch}"
               publicKeyToken="{token}"
               language="neutral"
               versionScope="nonSxS">
      <InputLocale>{lang}</InputLocale>
      <SystemLocale>{lang}</SystemLocale>
      <UILanguage>{lang}</UILanguage>
      <UserLocale>{lang}</UserLocale>
    </component>
"#
    );

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml += "<unattend xmlns=\"urn:schemas-microsoft-com:unattend\"\n";
    xml += "          xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\n";
    xml += &format!("  <settings pass=\"windowsPE\">\n{windows_pe}  </settings>\n");
    if !specialize.is_empty() {
        xml += &format!("  <settings pass=\"specialize\">\n{specialize}  </settings>\n");
    }
    xml += &format!("  <settings pass=\"oobeSystem\">\n{oobe_system}  </settings>\n");
    xml += "</unattend>\n";
    xml
}

/// Create an autounattend ISO from Windows install options JSON.
async fn create_autounattend_iso(
    env_dir: &Path,
    win_install_options_json: &str,
) -> Result<PathBuf, String> {
    let opts: WinInstallOptions = serde_json::from_str(win_install_options_json)
        .map_err(|e| format!("invalid win_install_options JSON: {e}"))?;

    let unattend_dir = env_dir.join("autounattend");
    tokio::fs::create_dir_all(&unattend_dir)
        .await
        .map_err(|e| format!("failed to create autounattend dir: {e}"))?;

    let xml = generate_autounattend_xml(&opts);
    let xml_path = unattend_dir.join("autounattend.xml");
    tokio::fs::write(&xml_path, &xml)
        .await
        .map_err(|e| format!("failed to write autounattend.xml: {e}"))?;

    let iso_path = env_dir.join("autounattend.iso");
    let iso_str = iso_path.to_string_lossy().to_string();
    let xml_path_str = xml_path.to_string_lossy().to_string();

    let output = Command::new("mkisofs")
        .args([
            "-output", &iso_str,
            "-volid", "OEMDRV",
            "-joliet",
            "-rock",
            &xml_path_str,
        ])
        .output()
        .await
        .map_err(|e| format!("failed to run mkisofs for autounattend: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("mkisofs autounattend failed: {}", stderr.trim()));
    }

    Ok(iso_path)
}

/// Download an ISO from a URL to the environment directory.
async fn download_iso(env_dir: &Path, url: &str) -> Result<PathBuf, String> {
    let iso_path = env_dir.join("install.iso");

    // If it's a local file path, symlink it
    if url.starts_with('/') || url.starts_with("file://") {
        let src = if let Some(stripped) = url.strip_prefix("file://") {
            stripped
        } else {
            url
        };
        if !Path::new(src).exists() {
            return Err(format!("ISO file not found: {src}"));
        }
        tokio::fs::symlink(src, &iso_path)
            .await
            .map_err(|e| format!("failed to symlink ISO: {e}"))?;
        return Ok(iso_path);
    }

    // Download via curl (handles HTTPS, redirects, large files efficiently)
    let iso_str = iso_path.to_string_lossy().to_string();
    let output = Command::new("curl")
        .args(["-fSL", "-o", &iso_str, url])
        .output()
        .await
        .map_err(|e| format!("failed to run curl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ISO download failed: {}", stderr.trim()));
    }

    Ok(iso_path)
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
            let is_windows = params.guest_os == "windows";
            let is_iso_install = params.iso_url.is_some();

            tracing::info!(
                env_id = %params.env_id,
                image = %params.image_name,
                guest_os = %params.guest_os,
                vcpus = params.vcpus,
                memory_bytes = params.memory_bytes,
                disk_bytes = params.disk_bytes,
                is_iso_install = is_iso_install,
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
                // Empty disk (fresh install from ISO or empty image)
                qemu_img(&["create", "-f", "qcow2", &disk_str, &size_str]).await?;
                tracing::info!(env_id = %params.env_id, size = %size_str, "created empty disk");
            }

            // Download installer ISO if provided
            let install_iso = if let Some(ref url) = params.iso_url {
                tracing::info!(env_id = %params.env_id, url = %url, "downloading installer ISO");
                Some(download_iso(&env_dir, url).await?)
            } else {
                None
            };

            // Generate autounattend ISO for Windows
            let autounattend_iso = if is_windows {
                if let Some(ref json) = params.win_install_options {
                    tracing::info!(env_id = %params.env_id, "generating autounattend ISO");
                    Some(create_autounattend_iso(&env_dir, json).await?)
                } else {
                    None
                }
            } else {
                None
            };

            // Generate cloud-init seed ISO (Linux only, not for Windows)
            let seed_iso = if !is_windows {
                create_cloud_init_seed(
                    &env_dir,
                    &params.env_id,
                    &params.ssh_authorized_keys,
                )
                .await
                .ok()
            } else {
                None
            };

            // Generate and write domain XML
            let dom_name = format!("orion-{}", params.env_id);
            let xml = generate_domain_xml(&DomainXmlOptions {
                name: &dom_name,
                vcpus: params.vcpus,
                memory_bytes: params.memory_bytes,
                disk_path: &disk_path,
                seed_iso_path: seed_iso.as_deref(),
                arch: &params.guest_arch,
                is_windows,
                install_iso_path: install_iso.as_deref(),
                autounattend_iso_path: autounattend_iso.as_deref(),
            });

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

            // Wait for VM to obtain an IP address (up to 30s).
            // Windows installs take much longer; still try so we record what we can.
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

            // Undefine, removing snapshots and NVRAM
            let _ = virsh(&uri, &["undefine", &dom_name, "--snapshots-metadata", "--nvram"]).await;

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

// ── USB passthrough helpers ─────────────────────────────────────────

/// Parse lsusb output into (bus, device, vendor_id, product_id, description).
pub fn parse_lsusb(output: &str) -> Vec<(String, String, String, String, String)> {
    let mut devices = Vec::new();
    for line in output.lines() {
        // Format: "Bus 001 Device 003: ID 1d6b:0002 Linux Foundation 2.0 root hub"
        let Some((prefix, rest)) = line.split_once(": ID ") else {
            continue;
        };
        let parts: Vec<&str> = prefix.split_whitespace().collect();
        if parts.len() < 4 {
            continue;
        }
        let bus = parts[1].to_string();
        let device = parts[3].to_string();

        let (id_part, description) = rest
            .split_once(' ')
            .unwrap_or((rest, "Unknown device"));
        let Some((vendor_id, product_id)) = id_part.split_once(':') else {
            continue;
        };
        devices.push((
            bus,
            device,
            vendor_id.to_string(),
            product_id.to_string(),
            description.to_string(),
        ));
    }
    devices
}

/// List USB devices on the host.
pub async fn list_host_usb_devices() -> Result<Vec<(String, String, String, String, String)>, String> {
    let output = Command::new("lsusb")
        .output()
        .await
        .map_err(|e| format!("failed to run lsusb: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lsusb failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(parse_lsusb(&String::from_utf8_lossy(&output.stdout)))
}

fn usb_hostdev_xml(vendor_id: &str, product_id: &str) -> String {
    format!(
        r#"<hostdev mode='subsystem' type='usb' managed='yes'>
  <source>
    <vendor id='0x{vendor_id}'/>
    <product id='0x{product_id}'/>
  </source>
</hostdev>"#
    )
}

/// Attach a USB device to a running domain.
pub async fn attach_usb(
    uri: &str,
    domain: &str,
    vendor_id: &str,
    product_id: &str,
) -> Result<(), String> {
    let xml = usb_hostdev_xml(vendor_id, product_id);
    let tmp_path = format!(
        "/tmp/orion-usb-{}.xml",
        uuid::Uuid::new_v4()
    );
    tokio::fs::write(&tmp_path, &xml)
        .await
        .map_err(|e| format!("write xml: {e}"))?;
    let result = virsh(uri, &["attach-device", domain, &tmp_path, "--live"]).await;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    result.map(|_| ())
}

/// Detach a USB device from a running domain.
pub async fn detach_usb(
    uri: &str,
    domain: &str,
    vendor_id: &str,
    product_id: &str,
) -> Result<(), String> {
    let xml = usb_hostdev_xml(vendor_id, product_id);
    let tmp_path = format!(
        "/tmp/orion-usb-{}.xml",
        uuid::Uuid::new_v4()
    );
    tokio::fs::write(&tmp_path, &xml)
        .await
        .map_err(|e| format!("write xml: {e}"))?;
    let result = virsh(uri, &["detach-device", domain, &tmp_path, "--live"]).await;
    let _ = tokio::fs::remove_file(&tmp_path).await;
    result.map(|_| ())
}

/// Detach all USB devices for a domain. Best-effort; logs errors but continues.
pub async fn detach_all_usb(uri: &str, domain: &str, devices: &[(String, String)]) {
    for (vid, pid) in devices {
        if let Err(e) = detach_usb(uri, domain, vid, pid).await {
            tracing::warn!(domain, vendor_id = %vid, product_id = %pid, error = %e, "failed to detach USB during cleanup");
        }
    }
}

/// Map an environment ID to its libvirt domain name.
pub fn env_domain_name(env_id: &str) -> String {
    format!("orion-{env_id}")
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
        let xml = generate_domain_xml(&DomainXmlOptions {
            name: "orion-test",
            vcpus: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024, // 8 GB
            disk_path: Path::new("/data/test/disk.qcow2"),
            seed_iso_path: None,
            arch: "x86_64",
            is_windows: false,
            install_iso_path: None,
            autounattend_iso_path: None,
        });
        assert!(xml.contains("<name>orion-test</name>"));
        assert!(xml.contains("<vcpu>4</vcpu>"));
        assert!(xml.contains("<memory unit='KiB'>8388608</memory>"));
        assert!(xml.contains("/data/test/disk.qcow2"));
        assert!(xml.contains("arch='x86_64'"));
        assert!(xml.contains("bus='virtio'"));
    }

    #[test]
    fn test_generate_domain_xml_windows() {
        let xml = generate_domain_xml(&DomainXmlOptions {
            name: "orion-win",
            vcpus: 4,
            memory_bytes: 8 * 1024 * 1024 * 1024,
            disk_path: Path::new("/data/win/disk.qcow2"),
            seed_iso_path: None,
            arch: "x86_64",
            is_windows: true,
            install_iso_path: Some(Path::new("/data/win/install.iso")),
            autounattend_iso_path: Some(Path::new("/data/win/autounattend.iso")),
        });
        assert!(xml.contains("<name>orion-win</name>"));
        // Windows ISO install uses SATA bus
        assert!(xml.contains("bus='sata'"));
        // Boot from cdrom first
        assert!(xml.contains("<boot dev='cdrom'/>"));
        // Hyper-V enlightenments
        assert!(xml.contains("<hyperv"));
        // QXL video
        assert!(xml.contains("type='qxl'"));
        // USB tablet
        assert!(xml.contains("type='tablet'"));
        // Both ISOs attached
        assert!(xml.contains("/data/win/install.iso"));
        assert!(xml.contains("/data/win/autounattend.iso"));
    }

    #[test]
    fn test_generate_autounattend_xml() {
        let opts = WinInstallOptions {
            bypass_tpm: Some(true),
            bypass_secure_boot: Some(true),
            bypass_ram: None,
            bypass_cpu: None,
            language: Some("en-US".into()),
            timezone: Some("UTC".into()),
            username: Some("admin".into()),
            password: Some("pass123".into()),
            auto_login: Some(true),
            auto_partition: Some(true),
            product_key: None,
            skip_oobe: Some(true),
        };
        let xml = generate_autounattend_xml(&opts);
        assert!(xml.contains("BypassTPMCheck"));
        assert!(xml.contains("BypassSecureBootCheck"));
        assert!(!xml.contains("BypassRAMCheck"));
        assert!(xml.contains("<WillWipeDisk>true</WillWipeDisk>"));
        assert!(xml.contains("<Name>admin</Name>"));
        assert!(xml.contains("<Value>pass123</Value>"));
        assert!(xml.contains("<AutoLogon>"));
        assert!(xml.contains("<SkipMachineOOBE>true</SkipMachineOOBE>"));
        assert!(xml.contains("<TimeZone>UTC</TimeZone>"));
    }

    #[test]
    fn test_parse_lsusb() {
        let output = "Bus 001 Device 001: ID 1d6b:0002 Linux Foundation 2.0 root hub\n\
                       Bus 002 Device 003: ID 046d:c077 Logitech, Inc. Mouse\n";
        let devices = parse_lsusb(output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0], ("001".into(), "001".into(), "1d6b".into(), "0002".into(), "Linux Foundation 2.0 root hub".into()));
        assert_eq!(devices[1], ("002".into(), "003".into(), "046d".into(), "c077".into(), "Logitech, Inc. Mouse".into()));
    }

    #[test]
    fn test_parse_lsusb_empty() {
        assert!(parse_lsusb("").is_empty());
    }

    #[test]
    fn test_env_domain_name() {
        assert_eq!(env_domain_name("abc-123"), "orion-abc-123");
    }
}
