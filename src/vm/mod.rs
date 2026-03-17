pub mod libvirt;
pub mod stub;

use std::future::Future;
use std::pin::Pin;

/// Information about a running VM
#[derive(Debug, Clone)]
pub struct VmInfo {
    pub provider_id: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub vnc_host: Option<String>,
    pub vnc_port: Option<u16>,
}

/// Parameters for creating a VM
#[derive(Debug, Clone)]
pub struct VmCreateParams {
    pub env_id: String,
    pub image_name: String,
    pub guest_os: String,
    pub guest_arch: String,
    pub node_host: String,
    pub vcpus: i64,
    pub memory_bytes: i64,
    pub disk_bytes: i64,
    /// SSH public keys to inject via cloud-init
    pub ssh_authorized_keys: Vec<String>,
    /// ISO URL for install-from-ISO creation (no base image)
    pub iso_url: Option<String>,
    /// Windows unattended install options (serialized JSON)
    pub win_install_options: Option<String>,
}

/// Trait for VM provider backends (libvirt, QEMU direct, etc.)
pub trait VmProvider: Send + Sync {
    fn create_vm(
        &self,
        params: VmCreateParams,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>>;

    fn destroy_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn suspend_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn resume_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn reboot_vm(
        &self,
        provider_id: &str,
        force: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn get_vm_info(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>>;

    fn create_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn delete_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn restore_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;

    fn migrate_vm(
        &self,
        provider_id: &str,
        target_host: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>>;
}
