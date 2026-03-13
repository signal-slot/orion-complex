import Foundation
@preconcurrency import Virtualization
import Logging

/// Manages macOS and Linux guest VMs using Apple's Virtualization.framework.
final class VMManager {
    private let bundleStorePath: String
    private let logger = Logger(label: "orion.node-agent.vm")

    /// Active VMs keyed by environment ID.
    private var vms: [String: RunningVM] = [:]

    struct RunningVM {
        let vm: VZVirtualMachine
        let bundlePath: String
        var macAddressString: String?
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

    // MARK: - VM lifecycle

    func createVM(envId: String, cpuCount: Int = 4, memoryGB: Int = 8, guestOS: String = "macos") async throws {
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

        // Network (NAT)
        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
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

        // Network (NAT)
        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
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

    func destroyVM(envId: String) async throws {
        guard let running = vms.removeValue(forKey: envId) else {
            throw VMError.notFound(envId)
        }

        logger.info("destroying macOS VM for environment \(envId)")

        if running.vm.canRequestStop {
            try running.vm.requestStop()
            try await Task.sleep(nanoseconds: 5_000_000_000)
        }

        if running.vm.state != .stopped {
            try await stopVMOnMain(running.vm)
        }

        // Remove the bundle directory
        try FileManager.default.removeItem(atPath: running.bundlePath)
        logger.info("macOS VM destroyed for environment \(envId)")
    }

    func suspendVM(envId: String) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("suspending macOS VM for environment \(envId)")
        try await pauseVMOnMain(running.vm)
    }

    func resumeVM(envId: String) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("resuming macOS VM for environment \(envId)")
        try await resumeVMOnMain(running.vm)
    }

    func rebootVM(envId: String, force: Bool) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        if force {
            logger.info("force rebooting macOS VM for environment \(envId)")
            try await stopVMOnMain(running.vm)
            try await startVMOnMain(running.vm)
        } else {
            logger.info("rebooting macOS VM for environment \(envId)")
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
        return vms[envId]?.vm.state
    }

    func activeVMCount() -> Int {
        return vms.count
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

    var description: String {
        switch self {
        case .notFound(let id): return "VM not found: \(id)"
        case .diskCreationFailed(let path): return "failed to create disk image: \(path)"
        case .invalidHardwareModel: return "invalid hardware model data"
        case .noSupportedHardwareModel: return "no supported macOS hardware model found"
        case .invalidMachineIdentifier: return "invalid machine identifier data"
        case .snapshotNotFound(let id): return "snapshot not found: \(id)"
        case .unsupportedGuestOS(let os): return "unsupported guest OS: \(os)"
        }
    }
}
