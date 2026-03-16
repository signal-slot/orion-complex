import Foundation
@preconcurrency import Virtualization
import Logging

/// Manages macOS and Linux guest VMs using Apple's Virtualization.framework.
final class VMManager {
    private let bundleStorePath: String
    private let logger = Logger(label: "orion.node-agent.vm")

    /// Active Virtualization.framework VMs keyed by environment ID.
    private var vms: [String: RunningVM] = [:]

    /// Active QEMU VMs keyed by environment ID.
    private var qemuVMs: [String: RunningQEMUVM] = [:]

    struct RunningVM {
        let vm: VZVirtualMachine
        let bundlePath: String
        var macAddressString: String?
    }

    struct RunningQEMUVM {
        let process: Process
        let bundlePath: String
        let monitorSocketPath: String
        let sshHostPort: Int
        let vncHostPort: Int
    }

    init(bundleStorePath: String) {
        self.bundleStorePath = bundleStorePath

        // Ensure bundle store directory exists
        try? FileManager.default.createDirectory(
            atPath: bundleStorePath,
            withIntermediateDirectories: true
        )
    }

    func bundlePath(envId: String) -> String {
        return "\(bundleStorePath)/\(envId).bundle"
    }

    /// Get the MAC address of a running VM (for DHCP lease matching).
    func macAddress(envId: String) -> String? {
        return vms[envId]?.macAddressString
    }

    /// Check if a VM is a QEMU-based VM.
    func isQEMUVM(envId: String) -> Bool {
        return qemuVMs[envId] != nil
    }

    /// Get the SSH host port for a QEMU VM (QEMU handles its own port forwarding).
    func qemuSSHPort(envId: String) -> Int? {
        return qemuVMs[envId]?.sshHostPort
    }

    /// Get the VNC host port for a QEMU VM.
    func qemuVNCPort(envId: String) -> Int? {
        return qemuVMs[envId]?.vncHostPort
    }

    // MARK: - VM lifecycle

    func createVM(envId: String, cpuCount: Int = 4, memoryGB: Int = 8, guestOS: String = "macos", guestArch: String? = nil) async throws {
        // If guest arch is x86_64 on arm64 host, use QEMU emulation
        if guestArch == "x86_64" {
            // QEMU TCG emulation is slow; use smaller defaults
            let qemuCPUs = min(cpuCount, 2)
            let qemuMemGB = min(memoryGB, 2)
            try await createQEMUVM(envId: envId, cpuCount: qemuCPUs, memoryGB: qemuMemGB)
            return
        }
        switch guestOS {
        case "macos":
            try await createMacOSVM(envId: envId, cpuCount: cpuCount, memoryGB: memoryGB)
        case "linux":
            try await createLinuxVM(envId: envId, cpuCount: cpuCount, memoryGB: memoryGB)
        default:
            throw VMError.unsupportedGuestOS(guestOS)
        }
    }

    // MARK: - macOS VM creation

    private func createMacOSVM(envId: String, cpuCount: Int, memoryGB: Int) async throws {
        logger.info("creating macOS VM for environment \(envId)")

        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        try FileManager.default.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        let diskPath = "\(bundlePath)/disk.img"
        let auxStoragePath = "\(bundlePath)/aux-storage"

        // Create a 64GB disk image if it doesn't exist
        if !FileManager.default.fileExists(atPath: diskPath) {
            let diskSize: Int64 = 64 * 1024 * 1024 * 1024
            try createDiskImage(path: diskPath, size: diskSize)
        }

        // Configure the VM
        let config = VZVirtualMachineConfiguration()

        // Platform: macOS
        let platform = VZMacPlatformConfiguration()

        // Load or create hardware model and machine identifier
        let hardwareModel = try loadOrCreateHardwareModel(bundlePath: bundlePath)
        platform.hardwareModel = hardwareModel

        let machineIdentifier = try loadOrCreateMachineIdentifier(bundlePath: bundlePath)
        platform.machineIdentifier = machineIdentifier

        // Auxiliary storage (NVRAM)
        if !FileManager.default.fileExists(atPath: auxStoragePath) {
            let _ = try VZMacAuxiliaryStorage(
                creatingStorageAt: URL(fileURLWithPath: auxStoragePath),
                hardwareModel: hardwareModel,
                options: []
            )
        }
        platform.auxiliaryStorage = VZMacAuxiliaryStorage(
            contentsOf: URL(fileURLWithPath: auxStoragePath)
        )

        config.platform = platform

        // Boot loader
        config.bootLoader = VZMacOSBootLoader()

        // CPU & memory
        config.cpuCount = max(cpuCount, VZVirtualMachineConfiguration.minimumAllowedCPUCount)
        config.memorySize = UInt64(memoryGB) * 1024 * 1024 * 1024
        config.memorySize = max(
            config.memorySize,
            VZVirtualMachineConfiguration.minimumAllowedMemorySize
        )

        // Storage
        let diskAttachment = try VZDiskImageStorageDeviceAttachment(
            url: URL(fileURLWithPath: diskPath),
            readOnly: false
        )
        config.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: diskAttachment)]

        // Network (NAT) — persist MAC address so DHCP lease survives VM restarts
        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
        let macFilePath = "\(bundlePath)/mac-address"
        if let savedMAC = try? String(contentsOfFile: macFilePath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
           let mac = VZMACAddress(string: savedMAC) {
            networkConfig.macAddress = mac
            logger.info("using saved MAC address: \(savedMAC)")
        } else {
            let newMAC = networkConfig.macAddress.string
            try? newMAC.write(toFile: macFilePath, atomically: true, encoding: .utf8)
            logger.info("generated new MAC address: \(newMAC)")
        }
        let macMAC = networkConfig.macAddress.string
        config.networkDevices = [networkConfig]

        // Keyboard & pointing device
        config.keyboards = [VZUSBKeyboardConfiguration()]
        config.pointingDevices = [VZUSBScreenCoordinatePointingDeviceConfiguration()]

        // Graphics
        let graphicsConfig = VZMacGraphicsDeviceConfiguration()
        graphicsConfig.displays = [
            VZMacGraphicsDisplayConfiguration(
                widthInPixels: 1920,
                heightInPixels: 1200,
                pixelsPerInch: 144
            )
        ]
        config.graphicsDevices = [graphicsConfig]

        // Entropy (random number generator)
        config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        // Shared directory for guest agent communication
        let sharedDir = VZVirtioFileSystemDeviceConfiguration(tag: "orion-shared")
        let sharedPath = "\(bundlePath)/shared"
        try FileManager.default.createDirectory(atPath: sharedPath, withIntermediateDirectories: true)
        sharedDir.share = VZSingleDirectoryShare(
            directory: VZSharedDirectory(url: URL(fileURLWithPath: sharedPath), readOnly: false)
        )
        config.directorySharingDevices = [sharedDir]

        try config.validate()

        // VZVirtualMachine must be created and used on the main queue
        let vm = await MainActor.run {
            VZVirtualMachine(configuration: config)
        }

        try await startVMOnMain(vm)
        logger.info("macOS VM started for environment \(envId)")

        vms[envId] = RunningVM(vm: vm, bundlePath: bundlePath, macAddressString: macMAC)
    }

    // MARK: - Linux VM creation

    private func createLinuxVM(envId: String, cpuCount: Int, memoryGB: Int, diskSizeGB: Int = 64) async throws {
        logger.info("creating Linux VM for environment \(envId)")

        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        try FileManager.default.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        let diskPath = "\(bundlePath)/disk.img"
        let efiVariableStorePath = "\(bundlePath)/efi-variable-store"

        // Create a disk image if it doesn't exist
        if !FileManager.default.fileExists(atPath: diskPath) {
            let diskSize: Int64 = Int64(diskSizeGB) * 1024 * 1024 * 1024
            try createDiskImage(path: diskPath, size: diskSize)
        }

        // Configure the VM
        let config = VZVirtualMachineConfiguration()

        // Platform: Generic (Linux)
        config.platform = VZGenericPlatformConfiguration()

        // Boot loader: EFI
        let efiBootLoader = VZEFIBootLoader()
        if FileManager.default.fileExists(atPath: efiVariableStorePath) {
            efiBootLoader.variableStore = VZEFIVariableStore(
                url: URL(fileURLWithPath: efiVariableStorePath)
            )
        } else {
            efiBootLoader.variableStore = try VZEFIVariableStore(
                creatingVariableStoreAt: URL(fileURLWithPath: efiVariableStorePath)
            )
        }
        config.bootLoader = efiBootLoader

        // CPU & memory
        config.cpuCount = max(cpuCount, VZVirtualMachineConfiguration.minimumAllowedCPUCount)
        config.memorySize = UInt64(memoryGB) * 1024 * 1024 * 1024
        config.memorySize = max(
            config.memorySize,
            VZVirtualMachineConfiguration.minimumAllowedMemorySize
        )

        // Storage
        let diskAttachment = try VZDiskImageStorageDeviceAttachment(
            url: URL(fileURLWithPath: diskPath),
            readOnly: false
        )
        config.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: diskAttachment)]

        // Network (NAT) — persist MAC address so DHCP lease survives VM restarts
        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
        let macFilePath = "\(bundlePath)/mac-address"
        if let savedMAC = try? String(contentsOfFile: macFilePath, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
           let mac = VZMACAddress(string: savedMAC) {
            networkConfig.macAddress = mac
            logger.info("using saved MAC address: \(savedMAC)")
        } else {
            let newMAC = networkConfig.macAddress.string
            try? newMAC.write(toFile: macFilePath, atomically: true, encoding: .utf8)
            logger.info("generated new MAC address: \(newMAC)")
        }
        let linuxMAC = networkConfig.macAddress.string
        config.networkDevices = [networkConfig]

        // Keyboard & pointing device
        config.keyboards = [VZUSBKeyboardConfiguration()]
        config.pointingDevices = [VZUSBScreenCoordinatePointingDeviceConfiguration()]

        // Graphics: Virtio GPU
        let graphicsConfig = VZVirtioGraphicsDeviceConfiguration()
        graphicsConfig.scanouts = [
            VZVirtioGraphicsScanoutConfiguration(widthInPixels: 1920, heightInPixels: 1080)
        ]
        config.graphicsDevices = [graphicsConfig]

        // Entropy (random number generator)
        config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        // Shared directory for guest agent communication
        let sharedDir = VZVirtioFileSystemDeviceConfiguration(tag: "orion-shared")
        let sharedPath = "\(bundlePath)/shared"
        try FileManager.default.createDirectory(atPath: sharedPath, withIntermediateDirectories: true)
        sharedDir.share = VZSingleDirectoryShare(
            directory: VZSharedDirectory(url: URL(fileURLWithPath: sharedPath), readOnly: false)
        )
        config.directorySharingDevices = [sharedDir]

        try config.validate()

        // VZVirtualMachine must be created and used on the main queue
        let vm = await MainActor.run {
            VZVirtualMachine(configuration: config)
        }

        try await startVMOnMain(vm)
        logger.info("Linux VM started for environment \(envId)")

        vms[envId] = RunningVM(vm: vm, bundlePath: bundlePath, macAddressString: linuxMAC)
    }

    // MARK: - QEMU x86-64 VM creation

    /// Next available host port for QEMU SSH forwarding.
    private static var nextQEMUSSHPort = 12022
    /// Next available host port for QEMU VNC.
    private static var nextQEMUVNCPort = 15950

    private func createQEMUVM(envId: String, cpuCount: Int, memoryGB: Int) async throws {
        logger.info("creating QEMU x86-64 VM for environment \(envId)")

        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        try FileManager.default.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        let diskPath = "\(bundlePath)/disk.img"
        let monitorSocket = "\(bundlePath)/monitor.sock"
        let sharedPath = "\(bundlePath)/shared"
        try FileManager.default.createDirectory(atPath: sharedPath, withIntermediateDirectories: true)

        // Disk image must already exist (cloned from template)
        guard FileManager.default.fileExists(atPath: diskPath) else {
            throw VMError.diskCreationFailed(diskPath)
        }

        // Allocate host ports for SSH and VNC forwarding
        let sshPort = VMManager.nextQEMUSSHPort
        VMManager.nextQEMUSSHPort += 1
        let vncPort = VMManager.nextQEMUVNCPort
        VMManager.nextQEMUVNCPort += 1

        // Build cloud-init ISO if seed files exist
        let ciISOPath = "\(bundlePath)/cidata.iso"
        let metaDataPath = "\(sharedPath)/meta-data"
        let userDataPath = "\(sharedPath)/user-data"
        if FileManager.default.fileExists(atPath: metaDataPath) &&
           FileManager.default.fileExists(atPath: userDataPath) {
            try buildCloudInitISO(
                metaDataPath: metaDataPath,
                userDataPath: userDataPath,
                outputPath: ciISOPath
            )
            logger.info("[\(envId)] cloud-init ISO created")
        }

        // Build QEMU command
        let qemuPath = "/opt/homebrew/bin/qemu-system-x86_64"
        let firmwarePath = "/opt/homebrew/share/qemu/edk2-x86_64-code.fd"
        let efiVarsPath = "\(bundlePath)/efi-vars.fd"

        // Create EFI variable store from template if it doesn't exist
        if !FileManager.default.fileExists(atPath: efiVarsPath) {
            let efiVarsTemplate = "/opt/homebrew/share/qemu/edk2-i386-vars.fd"
            if FileManager.default.fileExists(atPath: efiVarsTemplate) {
                try FileManager.default.copyItem(atPath: efiVarsTemplate, toPath: efiVarsPath)
            } else {
                // Create empty vars file (128KB)
                try createDiskImage(path: efiVarsPath, size: 131072)
            }
        }

        // Remove stale monitor socket
        try? FileManager.default.removeItem(atPath: monitorSocket)

        let memoryMB = max(memoryGB * 1024, 512)  // minimum 512MB

        var args = [
            "-machine", "q35",
            "-accel", "tcg",
            "-cpu", "max",
            "-smp", "\(cpuCount)",
            "-m", "\(memoryMB)M",
            "-drive", "if=pflash,format=raw,readonly=on,file=\(firmwarePath)",
            "-drive", "if=pflash,format=raw,file=\(efiVarsPath)",
            "-drive", "file=\(diskPath),format=qcow2,if=virtio",
            "-netdev", "user,id=net0,hostfwd=tcp::\(sshPort)-:22",
            "-device", "virtio-net-pci,netdev=net0",
            "-display", "none",
            "-vnc", "127.0.0.1:\(vncPort - 5900)",
            "-device", "virtio-gpu-pci",
            "-serial", "mon:stdio",
            "-monitor", "unix:\(monitorSocket),server,nowait",
            "-device", "virtio-rng-pci",
        ]

        // Attach install ISO if present (for ISO-based installs)
        let installISOPath = "\(bundlePath)/install.iso"
        if FileManager.default.fileExists(atPath: installISOPath) {
            args += ["-cdrom", installISOPath, "-boot", "d"]
        }

        // Attach cloud-init ISO if present
        if FileManager.default.fileExists(atPath: ciISOPath) {
            args += ["-drive", "file=\(ciISOPath),format=raw,if=virtio,readonly=on"]
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: qemuPath)
        process.arguments = args
        // Capture stderr for debugging
        let errPipe = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = errPipe
        try process.run()

        // Wait a moment for QEMU to start
        try await Task.sleep(nanoseconds: 3_000_000_000)

        guard process.isRunning else {
            let errData = errPipe.fileHandleForReading.availableData
            let errMsg = String(data: errData, encoding: .utf8) ?? ""
            logger.error("QEMU stderr: \(errMsg)")
            throw VMError.qemuStartFailed("\(envId): \(errMsg)")
        }

        qemuVMs[envId] = RunningQEMUVM(
            process: process,
            bundlePath: bundlePath,
            monitorSocketPath: monitorSocket,
            sshHostPort: sshPort,
            vncHostPort: vncPort
        )

        logger.info("QEMU x86-64 VM started for environment \(envId) (SSH port \(sshPort), VNC port \(vncPort))")
    }

    /// Build a cloud-init NoCloud ISO from meta-data and user-data files.
    private func buildCloudInitISO(metaDataPath: String, userDataPath: String, outputPath: String) throws {
        // Use mkisofs/genisoimage via hdiutil on macOS
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
        process.arguments = [
            "makehybrid",
            "-o", outputPath,
            "-joliet",
            "-iso",
            "-default-volume-name", "cidata",
            URL(fileURLWithPath: metaDataPath).deletingLastPathComponent().path
        ]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        process.waitUntilExit()
        if process.terminationStatus != 0 {
            let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            throw VMError.cloudInitISOFailed(output)
        }
    }

    /// Send a command to a QEMU monitor socket.
    private func sendQEMUMonitorCommand(socketPath: String, command: String) throws {
        let sock = socket(AF_UNIX, SOCK_STREAM, 0)
        guard sock >= 0 else { return }
        defer { close(sock) }

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: Int(104)) { dest in
                for i in 0..<min(pathBytes.count, 104) {
                    dest[i] = pathBytes[i]
                }
            }
        }

        let connectResult = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                connect(sock, sockPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connectResult == 0 else { return }

        // Read the greeting
        var buf = [UInt8](repeating: 0, count: 4096)
        _ = recv(sock, &buf, buf.count, 0)

        // Send command
        let cmd = command + "\n"
        _ = cmd.withCString { send(sock, $0, cmd.utf8.count, 0) }

        // Brief wait for command to process
        usleep(100_000)
    }

    func destroyVM(envId: String) async throws {
        // Handle QEMU VMs
        if let qemu = qemuVMs.removeValue(forKey: envId) {
            logger.info("destroying QEMU VM for environment \(envId)")
            try? sendQEMUMonitorCommand(socketPath: qemu.monitorSocketPath, command: "quit")
            qemu.process.terminate()
            qemu.process.waitUntilExit()
            try FileManager.default.removeItem(atPath: qemu.bundlePath)
            logger.info("QEMU VM destroyed for environment \(envId)")
            return
        }

        guard let running = vms.removeValue(forKey: envId) else {
            throw VMError.notFound(envId)
        }

        logger.info("destroying VM for environment \(envId)")

        if running.vm.canRequestStop {
            try running.vm.requestStop()
            // Wait up to 10s for graceful shutdown
            for _ in 0..<20 {
                try await Task.sleep(nanoseconds: 500_000_000)
                if running.vm.state == .stopped { break }
            }
        }

        if running.vm.state != .stopped {
            // Force stop with timeout
            do {
                try await withThrowingTaskGroup(of: Void.self) { group in
                    group.addTask {
                        try await self.stopVMOnMain(running.vm)
                    }
                    group.addTask {
                        try await Task.sleep(nanoseconds: 10_000_000_000)
                        throw VMError.notFound("force stop timeout")
                    }
                    // Wait for whichever finishes first
                    try await group.next()
                    group.cancelAll()
                }
            } catch {
                logger.warning("force stop timed out for \(envId), proceeding with cleanup")
            }
        }

        // Remove the bundle directory
        try FileManager.default.removeItem(atPath: running.bundlePath)
        logger.info("VM destroyed for environment \(envId)")
    }

    func suspendVM(envId: String) async throws {
        // Handle QEMU VMs
        if let qemu = qemuVMs[envId] {
            logger.info("suspending QEMU VM for environment \(envId)")
            try sendQEMUMonitorCommand(socketPath: qemu.monitorSocketPath, command: "stop")
            return
        }

        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("suspending VM for environment \(envId)")
        try await pauseVMOnMain(running.vm)
    }

    func resumeVM(envId: String) async throws {
        // Handle QEMU VMs
        if let qemu = qemuVMs[envId] {
            logger.info("resuming QEMU VM for environment \(envId)")
            try sendQEMUMonitorCommand(socketPath: qemu.monitorSocketPath, command: "cont")
            return
        }

        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("resuming VM for environment \(envId)")
        try await resumeVMOnMain(running.vm)
    }

    func rebootVM(envId: String, force: Bool) async throws {
        // Handle QEMU VMs
        if let qemu = qemuVMs[envId] {
            logger.info("rebooting QEMU VM for environment \(envId)")
            try sendQEMUMonitorCommand(socketPath: qemu.monitorSocketPath, command: "system_reset")
            return
        }

        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        if force {
            logger.info("force rebooting VM for environment \(envId)")
            try await stopVMOnMain(running.vm)
            try await startVMOnMain(running.vm)
        } else {
            logger.info("rebooting VM for environment \(envId)")
            if running.vm.canRequestStop {
                try running.vm.requestStop()
                try await Task.sleep(nanoseconds: 10_000_000_000)
                try await startVMOnMain(running.vm)
            }
        }
    }

    // MARK: - Snapshots

    func createSnapshot(envId: String, snapshotId: String) throws {
        guard vms[envId] != nil else {
            throw VMError.notFound(envId)
        }

        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        let snapshotsDir = "\(bundlePath)/snapshots"
        try FileManager.default.createDirectory(atPath: snapshotsDir, withIntermediateDirectories: true)

        let snapshotPath = "\(snapshotsDir)/\(snapshotId)"
        try FileManager.default.createDirectory(atPath: snapshotPath, withIntermediateDirectories: true)

        // Copy the disk image as the snapshot
        let diskPath = "\(bundlePath)/disk.img"
        let snapshotDisk = "\(snapshotPath)/disk.img"
        try FileManager.default.copyItem(atPath: diskPath, toPath: snapshotDisk)

        logger.info("snapshot \(snapshotId) created for environment \(envId)")
    }

    func deleteSnapshot(envId: String, snapshotId: String) throws {
        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        let snapshotPath = "\(bundlePath)/snapshots/\(snapshotId)"

        guard FileManager.default.fileExists(atPath: snapshotPath) else {
            throw VMError.snapshotNotFound(snapshotId)
        }

        try FileManager.default.removeItem(atPath: snapshotPath)
        logger.info("snapshot \(snapshotId) deleted for environment \(envId)")
    }

    func restoreSnapshot(envId: String, snapshotId: String) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        let snapshotDisk = "\(bundlePath)/snapshots/\(snapshotId)/disk.img"
        let diskPath = "\(bundlePath)/disk.img"

        guard FileManager.default.fileExists(atPath: snapshotDisk) else {
            throw VMError.snapshotNotFound(snapshotId)
        }

        logger.info("restoring snapshot \(snapshotId) for environment \(envId)")

        // Stop the VM, replace disk, restart
        try await stopVMOnMain(running.vm)

        try FileManager.default.removeItem(atPath: diskPath)
        try FileManager.default.copyItem(atPath: snapshotDisk, toPath: diskPath)

        try await startVMOnMain(running.vm)
        logger.info("snapshot \(snapshotId) restored for environment \(envId)")
    }

    // MARK: - Migration

    /// Export a VM bundle for migration to another node.
    /// Pauses and saves the VM state, then returns the bundle path.
    func exportForMigration(envId: String) async throws -> String {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("exporting VM \(envId) for migration")

        // VM should already be suspended/paused
        if running.vm.state == .running {
            try await pauseVMOnMain(running.vm)
        }

        // Save VM state to bundle (macOS 14+)
        let statePath = "\(running.bundlePath)/vm-state.dat"
        if #available(macOS 14.0, *) {
            try await running.vm.saveMachineStateTo(url: URL(fileURLWithPath: statePath))
        }

        try await stopVMOnMain(running.vm)
        vms.removeValue(forKey: envId)

        logger.info("VM \(envId) exported, bundle at \(running.bundlePath)")
        return running.bundlePath
    }

    /// Import a VM bundle that was migrated from another node.
    func importForMigration(envId: String, cpuCount: Int = 4, memoryGB: Int = 8, guestOS: String = "macos") async throws {
        logger.info("importing migrated VM \(envId)")

        // The bundle should already be at the expected path
        try await createVM(envId: envId, cpuCount: cpuCount, memoryGB: memoryGB, guestOS: guestOS)

        // If saved state exists, restore it and the VM will be paused
        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        let statePath = "\(bundlePath)/vm-state.dat"
        if FileManager.default.fileExists(atPath: statePath),
           let running = vms[envId] {
            if #available(macOS 14.0, *) {
                try await running.vm.restoreMachineStateFrom(url: URL(fileURLWithPath: statePath))
            }
            try FileManager.default.removeItem(atPath: statePath)
            logger.info("VM \(envId) imported with saved state")
        }
    }

    // MARK: - Guest provisioning

    /// Write SSH keys to the shared directory for the guest agent to pick up.
    func provisionSSHKeys(envId: String, username: String, keys: [String]) throws {
        let bundlePath = "\(bundleStorePath)/\(envId).bundle"
        let sharedPath = "\(bundlePath)/shared"

        try FileManager.default.createDirectory(atPath: sharedPath, withIntermediateDirectories: true)

        let request = ProvisioningCommand(
            action: "sync_ssh_keys",
            username: username,
            ssh_keys: keys
        )

        let data = try JSONEncoder().encode(request)
        let requestPath = "\(sharedPath)/provision-request.json"
        try data.write(to: URL(fileURLWithPath: requestPath))

        logger.info("wrote SSH key provisioning request for \(username) on env \(envId)")
    }

    private struct ProvisioningCommand: Encodable {
        let action: String
        let username: String?
        let ssh_keys: [String]?
    }

    func vmState(envId: String) -> VZVirtualMachine.State? {
        // QEMU VMs report as "running" if the process is alive
        if let qemu = qemuVMs[envId] {
            return qemu.process.isRunning ? .running : .stopped
        }
        return vms[envId]?.vm.state
    }

    func activeVMCount() -> Int {
        return vms.count + qemuVMs.count
    }

    // MARK: - Main queue helpers for Virtualization.framework

    private func startVMOnMain(_ vm: VZVirtualMachine) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            DispatchQueue.main.async {
                vm.start { result in
                    switch result {
                    case .success:
                        continuation.resume()
                    case .failure(let error):
                        continuation.resume(throwing: error)
                    }
                }
            }
        }
    }

    private func stopVMOnMain(_ vm: VZVirtualMachine) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            DispatchQueue.main.async {
                vm.stop { error in
                    if let error = error {
                        continuation.resume(throwing: error)
                    } else {
                        continuation.resume()
                    }
                }
            }
        }
    }

    private func pauseVMOnMain(_ vm: VZVirtualMachine) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            DispatchQueue.main.async {
                vm.pause { result in
                    switch result {
                    case .success:
                        continuation.resume()
                    case .failure(let error):
                        continuation.resume(throwing: error)
                    }
                }
            }
        }
    }

    private func resumeVMOnMain(_ vm: VZVirtualMachine) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            DispatchQueue.main.async {
                vm.resume { result in
                    switch result {
                    case .success:
                        continuation.resume()
                    case .failure(let error):
                        continuation.resume(throwing: error)
                    }
                }
            }
        }
    }

    // MARK: - Helpers

    private func createDiskImage(path: String, size: Int64) throws {
        let fd = open(path, O_RDWR | O_CREAT | O_TRUNC, 0o644)
        guard fd >= 0 else {
            throw VMError.diskCreationFailed(path)
        }
        let result = ftruncate(fd, off_t(size))
        close(fd)
        guard result == 0 else {
            throw VMError.diskCreationFailed(path)
        }
    }

    private func loadOrCreateHardwareModel(bundlePath: String) throws -> VZMacHardwareModel {
        let path = "\(bundlePath)/hardware-model.dat"
        if let data = FileManager.default.contents(atPath: path) {
            guard let model = VZMacHardwareModel(dataRepresentation: data) else {
                throw VMError.invalidHardwareModel
            }
            return model
        }

        // Hardware model should be created during IPSW restore (IPSWRestore.installMacOS).
        // If we reach here, the bundle was partially set up without a proper install.
        throw VMError.noSupportedHardwareModel
    }

    private func loadOrCreateMachineIdentifier(bundlePath: String) throws -> VZMacMachineIdentifier {
        let path = "\(bundlePath)/machine-id.dat"
        if let data = FileManager.default.contents(atPath: path) {
            guard let id = VZMacMachineIdentifier(dataRepresentation: data) else {
                throw VMError.invalidMachineIdentifier
            }
            return id
        }

        let id = VZMacMachineIdentifier()
        try id.dataRepresentation.write(to: URL(fileURLWithPath: path))
        return id
    }
}

enum VMError: Error, CustomStringConvertible {
    case notFound(String)
    case diskCreationFailed(String)
    case invalidHardwareModel
    case noSupportedHardwareModel
    case invalidMachineIdentifier
    case snapshotNotFound(String)
    case unsupportedGuestOS(String)
    case qemuStartFailed(String)
    case cloudInitISOFailed(String)

    var description: String {
        switch self {
        case .notFound(let id): return "VM not found: \(id)"
        case .diskCreationFailed(let path): return "failed to create disk image: \(path)"
        case .invalidHardwareModel: return "invalid hardware model data"
        case .noSupportedHardwareModel: return "no supported macOS hardware model found"
        case .invalidMachineIdentifier: return "invalid machine identifier data"
        case .snapshotNotFound(let id): return "snapshot not found: \(id)"
        case .unsupportedGuestOS(let os): return "unsupported guest OS: \(os)"
        case .qemuStartFailed(let id): return "QEMU failed to start for: \(id)"
        case .cloudInitISOFailed(let msg): return "failed to create cloud-init ISO: \(msg)"
        }
    }
}
