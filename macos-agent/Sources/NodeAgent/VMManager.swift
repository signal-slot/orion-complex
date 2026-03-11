import Foundation
import Virtualization
import Logging

/// Manages macOS guest VMs using Apple's Virtualization.framework.
final class VMManager {
    private let bundleStorePath: String
    private let logger = Logger(label: "orion.node-agent.vm")

    /// Active VMs keyed by environment ID.
    private var vms: [String: RunningVM] = [:]

    struct RunningVM {
        let vm: VZVirtualMachine
        let bundlePath: String
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

    // MARK: - VM lifecycle

    func createVM(envId: String, cpuCount: Int = 4, memoryGB: Int = 8) async throws {
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

        let vm = VZVirtualMachine(configuration: config)

        try await vm.start()
        logger.info("macOS VM started for environment \(envId)")

        vms[envId] = RunningVM(vm: vm, bundlePath: bundlePath)
    }

    func destroyVM(envId: String) async throws {
        guard let running = vms.removeValue(forKey: envId) else {
            throw VMError.notFound(envId)
        }

        logger.info("destroying macOS VM for environment \(envId)")

        if running.vm.canRequestStop {
            try running.vm.requestStop()
            // Wait briefly for graceful shutdown
            try await Task.sleep(nanoseconds: 5_000_000_000)
        }

        if running.vm.state != .stopped {
            try await running.vm.stop()
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
        try await running.vm.pause()
    }

    func resumeVM(envId: String) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        logger.info("resuming macOS VM for environment \(envId)")
        try await running.vm.resume()
    }

    func rebootVM(envId: String, force: Bool) async throws {
        guard let running = vms[envId] else {
            throw VMError.notFound(envId)
        }

        if force {
            logger.info("force rebooting macOS VM for environment \(envId)")
            try await running.vm.stop()
            try await running.vm.start()
        } else {
            logger.info("rebooting macOS VM for environment \(envId)")
            if running.vm.canRequestStop {
                try running.vm.requestStop()
                // Wait for shutdown, then restart
                try await Task.sleep(nanoseconds: 10_000_000_000)
                try await running.vm.start()
            }
        }
    }

    func vmState(envId: String) -> VZVirtualMachine.State? {
        return vms[envId]?.vm.state
    }

    func activeVMCount() -> Int {
        return vms.count
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

        // Use the host's most recent supported hardware model
        guard let model = VZMacHardwareModel.supportedHardwareModels.first else {
            throw VMError.noSupportedHardwareModel
        }
        try model.dataRepresentation.write(to: URL(fileURLWithPath: path))
        return model
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

    var description: String {
        switch self {
        case .notFound(let id): return "VM not found: \(id)"
        case .diskCreationFailed(let path): return "failed to create disk image: \(path)"
        case .invalidHardwareModel: return "invalid hardware model data"
        case .noSupportedHardwareModel: return "no supported macOS hardware model found"
        case .invalidMachineIdentifier: return "invalid machine identifier data"
        }
    }
}
