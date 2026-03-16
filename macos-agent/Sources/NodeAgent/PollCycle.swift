import Foundation
import Logging

enum NodeAgentHelpers {
    static func pollCycle(
        api: APIClient,
        vmManager: VMManager,
        ipswRestore: IPSWRestore,
        templateManager: TemplateManager,
        portForwarder: PortForwarder,
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

                    let guestOS = env.guest_os ?? "macos"
                    let guestArch = env.guest_arch ?? "arm64"
                    let isQEMU = guestArch == "x86_64"
                    logger.info("[\(env.id)] creating \(guestOS) VM (arch: \(guestArch)\(isQEMU ? ", QEMU" : ""))")
                    do {
                        let bundlePath = vmManager.bundlePath(envId: env.id)

                        if guestOS != "macos" {
                            if let isoURL = env.iso_url, !isoURL.isEmpty, isoURL != "pending-upload" {
                                // ISO-based install: download or link ISO and create blank disk
                                logger.info("[\(env.id)] ISO install from \(isoURL)")
                                try FileManager.default.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)
                                let isoPath = "\(bundlePath)/install.iso"
                                if !FileManager.default.fileExists(atPath: isoPath) {
                                    if isoURL.hasPrefix("/") || isoURL.hasPrefix("file://") {
                                        // Local file path — symlink to avoid copying large ISOs
                                        let localPath = isoURL.hasPrefix("file://")
                                            ? String(isoURL.dropFirst("file://".count))
                                            : isoURL
                                        guard FileManager.default.fileExists(atPath: localPath) else {
                                            throw VMError.qemuStartFailed("ISO file not found: \(localPath)")
                                        }
                                        try FileManager.default.createSymbolicLink(atPath: isoPath, withDestinationPath: localPath)
                                        logger.info("[\(env.id)] ISO linked from \(localPath)")
                                    } else {
                                        logger.info("[\(env.id)] downloading ISO...")
                                        let proc = Process()
                                        proc.executableURL = URL(fileURLWithPath: "/usr/bin/curl")
                                        proc.arguments = ["-L", "-o", isoPath, isoURL]
                                        try proc.run()
                                        proc.waitUntilExit()
                                        guard proc.terminationStatus == 0 else {
                                            throw VMError.qemuStartFailed("ISO download failed (exit \(proc.terminationStatus))")
                                        }
                                        logger.info("[\(env.id)] ISO downloaded")
                                    }
                                }
                                // Create blank qcow2 disk
                                let diskPath = "\(bundlePath)/disk.img"
                                if !FileManager.default.fileExists(atPath: diskPath) {
                                    let qemuImg = Process()
                                    qemuImg.executableURL = URL(fileURLWithPath: "/opt/homebrew/bin/qemu-img")
                                    qemuImg.arguments = ["create", "-f", "qcow2", diskPath, "64G"]
                                    try qemuImg.run()
                                    qemuImg.waitUntilExit()
                                    guard qemuImg.terminationStatus == 0 else {
                                        throw VMError.diskCreationFailed(diskPath)
                                    }
                                }
                                FileManager.default.createFile(atPath: "\(bundlePath)/installed", contents: nil)
                                try await vmManager.createVM(envId: env.id, guestOS: guestOS, guestArch: guestArch)
                            } else if let imageId = env.image_id, templateManager.hasTemplate(imageId: imageId, guestOS: "linux") {
                                // Clone from a pre-registered template
                                if isQEMU {
                                    logger.info("[\(env.id)] cloning QEMU template \(imageId)")
                                    try templateManager.cloneQEMUTemplate(imageId: imageId, toBundlePath: bundlePath)
                                } else {
                                    logger.info("[\(env.id)] cloning Linux template \(imageId)")
                                    try templateManager.cloneLinuxTemplate(imageId: imageId, toBundlePath: bundlePath)
                                }
                                logger.info("[\(env.id)] template cloned")
                            } else if env.iso_url == "pending-upload" {
                                // ISO upload still in progress, wait
                                continue
                            } else {
                                let imageDesc = env.image_id ?? "nil"
                                logger.error("[\(env.id)] no template for image \(imageDesc) — images must be pre-registered")
                                let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                                continue
                            }

                            // Write cloud-init seed files for Linux guest provisioning
                            // (skipped for non-Linux guests like Windows)
                            if guestOS == "linux" {
                                let sharedPath = "\(bundlePath)/shared"
                                try FileManager.default.createDirectory(
                                    atPath: sharedPath,
                                    withIntermediateDirectories: true
                                )

                                let shortId = String(env.id.prefix(8))
                                let hostname = "orion-\(shortId)"

                                var sshKeys: [String] = []
                                if let ownerId = env.owner_user_id {
                                    let keys = try await api.listUserSSHKeys(userId: ownerId)
                                    sshKeys = keys.compactMap(\.public_key)
                                }

                                // meta-data (instance identity)
                                let metaData = """
                                instance-id: \(env.id)
                                local-hostname: \(hostname)
                                """
                                try metaData.write(
                                    toFile: "\(sharedPath)/meta-data",
                                    atomically: true,
                                    encoding: .utf8
                                )

                                // user-data (cloud-init config)
                                var userData = """
                                #cloud-config
                                hostname: \(hostname)
                                manage_etc_hosts: true
                                """
                                if !sshKeys.isEmpty {
                                    userData += "\nssh_authorized_keys:\n"
                                    for key in sshKeys {
                                        userData += "  - \(key)\n"
                                    }
                                }
                                try userData.write(
                                    toFile: "\(sharedPath)/user-data",
                                    atomically: true,
                                    encoding: .utf8
                                )

                                // Write authorized_keys for the guest agent to sync
                                if !sshKeys.isEmpty {
                                    let keysContent = sshKeys.joined(separator: "\n") + "\n"
                                    try keysContent.write(
                                        toFile: "\(sharedPath)/authorized_keys",
                                        atomically: true,
                                        encoding: .utf8
                                    )
                                }

                                logger.info("[\(env.id)] cloud-init seed written: \(sshKeys.count) SSH key(s), hostname=\(hostname)")
                            }

                            try await vmManager.createVM(envId: env.id, guestOS: guestOS, guestArch: guestArch)
                        } else {
                            // macOS guest: IPSW restore or golden image clone
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

                            try await vmManager.createVM(envId: env.id, guestOS: "macos")
                        }

                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                        logger.info("[\(env.id)] VM running")

                        // Set up port forwarding if enabled, otherwise report internal IP
                        if env.port_forwarding == 1 {
                            await setupPortForwarding(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        } else {
                            await reportVMInternalEndpoints(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        }
                    } catch {
                        logger.error("[\(env.id)] failed to create VM: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                case "suspending":
                    portForwarder.stopForwarding(envId: env.id)
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
                        // Re-establish port forwarding if enabled, otherwise report internal IP
                        if env.port_forwarding == 1 {
                            await setupPortForwarding(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        } else {
                            await reportVMInternalEndpoints(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        }
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
                    portForwarder.stopForwarding(envId: env.id)
                    logger.info("[\(env.id)] destroying VM")
                    do {
                        try await vmManager.destroyVM(envId: env.id)
                    } catch {
                        logger.warning("[\(env.id)] VM destroy error (cleaning up anyway): \(error)")
                        // Force-remove the bundle even if VM stop failed
                        let bundlePath = vmManager.bundlePath(envId: env.id)
                        try? FileManager.default.removeItem(atPath: bundlePath)
                    }
                    // Always delete from DB regardless of VM stop result
                    do {
                        try await api.deleteEnvironment(envId: env.id)
                        logger.info("[\(env.id)] VM destroyed")
                    } catch {
                        logger.error("[\(env.id)] failed to delete environment from API: \(error)")
                    }

                case "running":
                    // Verify VM is actually alive — if agent restarted or VM crashed,
                    // vmState returns nil (not tracked) or .stopped/.error
                    let localState = vmManager.vmState(envId: env.id)
                    if localState == nil {
                        // VM not tracked by this agent — check if bundle exists
                        let bundlePath = vmManager.bundlePath(envId: env.id)
                        let bundleExists = FileManager.default.fileExists(atPath: bundlePath)
                        if bundleExists {
                            // Auto-recover: boot VM from existing bundle
                            logger.info("[\(env.id)] VM not running but bundle exists — auto-recovering")
                            let guestOS = env.guest_os ?? "macos"
                            let guestArch = env.guest_arch ?? "arm64"
                            do {
                                try await vmManager.createVM(envId: env.id, guestOS: guestOS, guestArch: guestArch)
                                logger.info("[\(env.id)] VM auto-recovered successfully")

                                // Set up port forwarding if enabled, otherwise report internal IP
                                if env.port_forwarding == 1 {
                                    await setupPortForwarding(
                                        envId: env.id,
                                        vmManager: vmManager,
                                        portForwarder: portForwarder,
                                        api: api,
                                        logger: logger
                                    )
                                } else {
                                    await reportVMInternalEndpoints(
                                        envId: env.id,
                                        vmManager: vmManager,
                                        portForwarder: portForwarder,
                                        api: api,
                                        logger: logger
                                    )
                                }
                            } catch {
                                logger.error("[\(env.id)] auto-recovery failed: \(error) — marking as failed")
                                let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                            }
                            continue
                        } else {
                            logger.warning("[\(env.id)] VM bundle missing — marking as failed")
                            let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                            continue
                        }
                    } else if localState == .stopped || localState == .error {
                        logger.warning("[\(env.id)] VM is \(localState == .stopped ? "stopped" : "in error state") — marking as failed")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                        continue
                    }

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

                    // Manage port forwarding based on the flag
                    if env.port_forwarding == 1 {
                        // Port forwarding enabled — ensure it's active
                        if !portForwarder.isForwarding(envId: env.id) {
                            await setupPortForwarding(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        }
                    } else {
                        // Port forwarding disabled — stop if active
                        if portForwarder.isForwarding(envId: env.id) {
                            portForwarder.stopForwarding(envId: env.id)
                            logger.info("[\(env.id)] port forwarding stopped (disabled by user)")
                        }
                        // Report VM internal IP so the backend WebSocket proxy can reach the VM
                        if env.ssh_host == nil || env.vnc_host == nil {
                            await reportVMInternalEndpoints(
                                envId: env.id,
                                vmManager: vmManager,
                                portForwarder: portForwarder,
                                api: api,
                                logger: logger
                            )
                        }
                    }

                case "capturing":
                    guard let captureImageId = env.capture_image_id else {
                        logger.error("[\(env.id)] capturing state but no capture_image_id")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                        continue
                    }
                    logger.info("[\(env.id)] capturing as image \(captureImageId)")
                    do {
                        let bundlePath = vmManager.bundlePath(envId: env.id)
                        let templateDir = templateManager.templatePath(imageId: captureImageId)
                        try FileManager.default.createDirectory(atPath: templateDir, withIntermediateDirectories: true)

                        // Copy disk image to template
                        let srcDisk = "\(bundlePath)/disk.img"
                        let dstDisk = "\(templateDir)/disk.img"
                        if FileManager.default.fileExists(atPath: dstDisk) {
                            try FileManager.default.removeItem(atPath: dstDisk)
                        }
                        try FileManager.default.copyItem(atPath: srcDisk, toPath: dstDisk)
                        FileManager.default.createFile(atPath: "\(templateDir)/installed", contents: nil)

                        logger.info("[\(env.id)] image captured to \(templateDir)")
                        // Transition back to running (or suspended)
                        let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                    } catch {
                        logger.error("[\(env.id)] capture failed: \(error)")
                        let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                    }

                default:
                    break
                }
            }
        } catch {
            logger.error("poll error: \(error)")
        }
    }

    // MARK: - Port forwarding helper

    private static func setupPortForwarding(
        envId: String,
        vmManager: VMManager,
        portForwarder: PortForwarder,
        api: APIClient,
        logger: Logger
    ) async {
        var vmIP: String?

        // Try MAC-based discovery first (works for VMs created in this session)
        if let macAddress = vmManager.macAddress(envId: envId) {
            vmIP = await portForwarder.discoverVMIP(macAddress: macAddress, retries: 10, interval: 2)
        }

        // Fallback: resolve by mDNS hostname (works for pre-existing VMs)
        if vmIP == nil {
            let shortId = String(envId.prefix(8))
            let hostname = "orion-\(shortId)"
            logger.info("[\(envId)] trying hostname resolution for \(hostname)")
            vmIP = await portForwarder.discoverVMIPByHostname(hostname: hostname, retries: 10, interval: 2)
        }

        guard let vmIP = vmIP else {
            logger.warning("[\(envId)] cannot set up port forwarding: VM IP not found")
            return
        }

        let (sshPort, vncPort) = portForwarder.startForwarding(envId: envId, vmIP: vmIP)
        let hostIP = PortForwarder.hostLANIP() ?? "127.0.0.1"
        do {
            let _ = try await api.updateEndpoints(
                envId: envId,
                endpoints: .init(
                    ssh_host: hostIP,
                    ssh_port: sshPort,
                    vnc_host: hostIP,
                    vnc_port: vncPort
                )
            )
            logger.info("[\(envId)] port forwarding reported: SSH=\(hostIP):\(sshPort), VNC=\(hostIP):\(vncPort)")
        } catch {
            logger.error("[\(envId)] failed to report endpoints: \(error)")
        }
    }

    // MARK: - VZ VM endpoint discovery (no port forwarding)

    private static func reportVMInternalEndpoints(
        envId: String,
        vmManager: VMManager,
        portForwarder: PortForwarder,
        api: APIClient,
        logger: Logger
    ) async {
        var vmIP: String?

        // Try MAC-based discovery first
        if let macAddress = vmManager.macAddress(envId: envId) {
            vmIP = await portForwarder.discoverVMIP(macAddress: macAddress, retries: 10, interval: 2)
        }

        // Fallback: resolve by mDNS hostname
        if vmIP == nil {
            let shortId = String(envId.prefix(8))
            let hostname = "orion-\(shortId)"
            logger.info("[\(envId)] trying hostname resolution for \(hostname)")
            vmIP = await portForwarder.discoverVMIPByHostname(hostname: hostname, retries: 10, interval: 2)
        }

        guard let vmIP = vmIP else {
            logger.warning("[\(envId)] cannot report endpoints: VM IP not found")
            return
        }

        // Report VM's internal IP directly — backend connects without port forwarding
        let _ = try? await api.updateEndpoints(
            envId: envId,
            endpoints: .init(
                ssh_host: vmIP,
                ssh_port: 22,
                vnc_host: vmIP,
                vnc_port: 5900
            )
        )
        logger.info("[\(envId)] VZ endpoints: SSH=\(vmIP):22, VNC=\(vmIP):5900")
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
