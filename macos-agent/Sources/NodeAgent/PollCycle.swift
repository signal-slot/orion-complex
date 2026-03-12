import Foundation
import Logging

enum NodeAgentHelpers {
    static func pollCycle(
        api: APIClient,
        vmManager: VMManager,
        ipswRestore: IPSWRestore,
        templateManager: TemplateManager,
        nodeId: String?,
        logger: Logger
    ) async {
        do {
            let environments = try await api.listEnvironments()

            for env in environments {
                // Only handle macOS environments assigned to this node
                guard env.provider == "macos" || env.provider == "virtualization" else { continue }
                if let nodeId = nodeId, env.node_id != nodeId { continue }

                switch env.state {
                case "creating":
                    // Skip if VM already exists locally
                    guard vmManager.vmState(envId: env.id) == nil else { continue }

                    logger.info("[\(env.id)] creating VM")
                    do {
                        let bundlePath = vmManager.bundlePath(envId: env.id)

                        // If not installed yet, try golden image clone, then fall back to IPSW restore
                        if !ipswRestore.isInstalled(bundlePath: bundlePath) {
                            if let imageId = env.image_id, templateManager.hasTemplate(imageId: imageId) {
                                logger.info("[\(env.id)] cloning from golden image \(imageId)")
                                try templateManager.cloneTemplate(imageId: imageId, toBundlePath: bundlePath)
                                logger.info("[\(env.id)] golden image cloned")
                            } else {
                                logger.info("[\(env.id)] no template available, performing IPSW restore...")
                                let (ipswURL, restoreImage) = try await ipswRestore.downloadLatestIPSW()
                                try await ipswRestore.installMacOS(
                                    bundlePath: bundlePath,
                                    ipswURL: ipswURL,
                                    restoreImage: restoreImage
                                )
                                logger.info("[\(env.id)] macOS installed successfully")
                            }

                            // Provision SSH keys and hostname on the disk before first boot
                            var sshKeys: [String] = []
                            if let ownerId = env.owner_user_id {
                                let keys = try await api.listUserSSHKeys(userId: ownerId)
                                sshKeys = keys.compactMap(\.public_key)
                            }
                            let shortId = String(env.id.prefix(8))
                            let hostname = "orion-\(shortId)"
                            logger.info("[\(env.id)] provisioning disk: \(sshKeys.count) SSH key(s), hostname=\(hostname)")
                            try templateManager.provisionDisk(
                                bundlePath: bundlePath,
                                authorizedKeys: sshKeys,
                                hostname: hostname
                            )
                        }

                        try await vmManager.createVM(envId: env.id)
                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                        logger.info("[\(env.id)] VM running")
                    } catch {
                        logger.error("[\(env.id)] failed to create VM: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "suspending":
                    logger.info("[\(env.id)] suspending VM")
                    do {
                        try await vmManager.suspendVM(envId: env.id)
                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "suspended")
                        logger.info("[\(env.id)] VM suspended")
                    } catch {
                        logger.error("[\(env.id)] failed to suspend: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "resuming":
                    logger.info("[\(env.id)] resuming VM")
                    do {
                        try await vmManager.resumeVM(envId: env.id)
                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                        logger.info("[\(env.id)] VM resumed")
                    } catch {
                        logger.error("[\(env.id)] failed to resume: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "rebooting":
                    logger.info("[\(env.id)] rebooting VM")
                    do {
                        try await vmManager.rebootVM(envId: env.id, force: false)
                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                        logger.info("[\(env.id)] VM rebooted")
                    } catch {
                        logger.error("[\(env.id)] failed to reboot: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "migrating":
                    // Source node: export the VM bundle for migration.
                    // The control plane has already updated node_id to the target.
                    logger.info("[\(env.id)] handling migration")
                    do {
                        let _ = try await vmManager.exportForMigration(envId: env.id)
                        logger.info("[\(env.id)] VM exported for migration")
                    } catch VMError.notFound(_) {
                        // VM not on this node — we are the target node, import it
                        if env.node_id == nodeId {
                            logger.info("[\(env.id)] importing migrated VM")
                            do {
                                try await vmManager.importForMigration(envId: env.id)
                                let _ = try await api.updateEnvironmentState(envId: env.id, state: "suspended")
                                logger.info("[\(env.id)] migration import complete")
                            } catch {
                                logger.error("[\(env.id)] failed to import: \(error)")
                                let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                            }
                        }
                    } catch {
                        logger.error("[\(env.id)] migration failed: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "destroying":
                    logger.info("[\(env.id)] destroying VM")
                    do {
                        try await vmManager.destroyVM(envId: env.id)
                        try await api.deleteEnvironment(envId: env.id)
                        logger.info("[\(env.id)] VM destroyed")
                    } catch {
                        logger.error("[\(env.id)] failed to destroy: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "running":
                    // For running VMs, provision SSH keys via the shared directory
                    if let ownerId = env.owner_user_id {
                        do {
                            let keys = try await api.listUserSSHKeys(userId: ownerId)
                            let publicKeys = keys.compactMap(\.public_key)
                            if !publicKeys.isEmpty {
                                try vmManager.provisionSSHKeys(
                                    envId: env.id,
                                    username: ownerId,
                                    keys: publicKeys
                                )
                            }
                        } catch {
                            // Non-fatal: SSH key provisioning is best-effort
                            logger.debug("[\(env.id)] SSH key provisioning skipped: \(error)")
                        }
                    }

                default:
                    break
                }
            }
        } catch {
            logger.error("poll error: \(error)")
        }
    }

    // MARK: - System info helpers

    static func currentArch() -> String {
        #if arch(arm64)
        return "arm64"
        #elseif arch(x86_64)
        return "x86_64"
        #else
        return "unknown"
        #endif
    }

    static func diskSpace() -> Int64 {
        let homeURL = URL(fileURLWithPath: NSHomeDirectory())
        guard let values = try? homeURL.resourceValues(forKeys: [.volumeTotalCapacityKey]),
              let total = values.volumeTotalCapacity else {
            return 0
        }
        return Int64(total)
    }
}
