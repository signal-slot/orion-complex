use std::future::Future;
use std::pin::Pin;

use super::{VmCreateParams, VmInfo, VmProvider};

/// Stub VM provider for testing and development.
/// Returns success for all operations without managing real VMs.
pub struct StubProvider;

impl Default for StubProvider {
    fn default() -> Self {
        Self
    }
}

impl StubProvider {
    pub fn new() -> Self {
        Self
    }
}

impl VmProvider for StubProvider {
    fn create_vm(
        &self,
        params: VmCreateParams,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        Box::pin(async move {
            tracing::info!(
                env_id = %params.env_id,
                image = %params.image_name,
                vcpus = params.vcpus,
                memory_bytes = params.memory_bytes,
                "creating VM (stub)"
            );
            let host = params.node_host;
            Ok(VmInfo {
                provider_id: format!("libvirt-{}", params.env_id),
                vnc_host: Some(host.clone()),
                ssh_host: host,
                ssh_port: 22,
                vnc_port: Some(5900),
            })
        })
    }

    fn destroy_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, "destroying VM (stub)");
            Ok(())
        })
    }

    fn suspend_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, "suspending VM (stub)");
            Ok(())
        })
    }

    fn resume_vm(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, "resuming VM (stub)");
            Ok(())
        })
    }

    fn reboot_vm(
        &self,
        provider_id: &str,
        force: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, force = force, "rebooting VM (stub)");
            Ok(())
        })
    }

    fn get_vm_info(
        &self,
        provider_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<VmInfo, String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        Box::pin(async move {
            Ok(VmInfo {
                provider_id: provider_id.clone(),
                ssh_host: "127.0.0.1".into(),
                ssh_port: 22,
                vnc_host: Some("127.0.0.1".into()),
                vnc_port: Some(5900),
            })
        })
    }

    fn create_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        let snapshot_name = snapshot_name.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, snapshot_name = %snapshot_name, "creating snapshot (stub)");
            Ok(())
        })
    }

    fn delete_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        let snapshot_name = snapshot_name.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, snapshot_name = %snapshot_name, "deleting snapshot (stub)");
            Ok(())
        })
    }

    fn restore_snapshot(
        &self,
        provider_id: &str,
        snapshot_name: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        let snapshot_name = snapshot_name.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, snapshot_name = %snapshot_name, "restoring snapshot (stub)");
            Ok(())
        })
    }

    fn migrate_vm(
        &self,
        provider_id: &str,
        target_host: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        let provider_id = provider_id.to_string();
        let target_host = target_host.to_string();
        Box::pin(async move {
            tracing::info!(provider_id = %provider_id, target_host = %target_host, "migrating VM (stub)");
            Ok(())
        })
    }
}
