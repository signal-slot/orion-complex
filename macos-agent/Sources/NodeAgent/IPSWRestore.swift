import Foundation
import Virtualization
import Logging

/// Handles downloading macOS IPSW restore images and performing initial OS installation.
final class IPSWRestore {
    private let bundleStorePath: String
    private let logger = Logger(label: "orion.node-agent.ipsw")

    init(bundleStorePath: String) {
        self.bundleStorePath = bundleStorePath
    }

    /// Path to the cached IPSW file for a given macOS version.
    func ipswPath(version: String) -> String {
        return "\(bundleStorePath)/ipsw/\(version).ipsw"
    }

    /// Download the latest macOS restore image compatible with this host.
    func downloadLatestIPSW() async throws -> (URL, VZMacOSRestoreImage) {
        logger.info("fetching latest supported macOS restore image info...")

        let restoreImage = try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<VZMacOSRestoreImage, Error>) in
            VZMacOSRestoreImage.fetchLatestSupported { result in
                switch result {
                case .success(let image):
                    continuation.resume(returning: image)
                case .failure(let error):
                    continuation.resume(throwing: error)
                }
            }
        }

        let buildVersion = restoreImage.buildVersion
        logger.info("latest restore image: build \(buildVersion)")

        let ipswDir = "\(bundleStorePath)/ipsw"
        try FileManager.default.createDirectory(atPath: ipswDir, withIntermediateDirectories: true)

        let destPath = "\(ipswDir)/\(buildVersion).ipsw"
        let destURL = URL(fileURLWithPath: destPath)

        // Check if already downloaded
        if FileManager.default.fileExists(atPath: destPath) {
            logger.info("IPSW already cached at \(destPath)")
            return (destURL, restoreImage)
        }

        // Download the IPSW
        logger.info("downloading IPSW from \(restoreImage.url)...")

        let (tempURL, response) = try await URLSession.shared.download(from: restoreImage.url)

        guard let httpResponse = response as? HTTPURLResponse,
              (200...299).contains(httpResponse.statusCode) else {
            throw IPSWError.downloadFailed("HTTP error downloading IPSW")
        }

        // Move to final location
        if FileManager.default.fileExists(atPath: destPath) {
            try FileManager.default.removeItem(atPath: destPath)
        }
        try FileManager.default.moveItem(at: tempURL, to: destURL)

        logger.info("IPSW downloaded to \(destPath)")
        return (destURL, restoreImage)
    }

    /// Install macOS from an IPSW into a VM bundle.
    /// This creates a new VM, performs the restore, then shuts it down.
    func installMacOS(
        bundlePath: String,
        ipswURL: URL,
        restoreImage: VZMacOSRestoreImage,
        cpuCount: Int = 4,
        memoryGB: Int = 8,
        diskSizeGB: Int = 64
    ) async throws {
        logger.info("starting macOS installation into \(bundlePath)")

        guard let hardwareModel = restoreImage.mostFeaturefulSupportedConfiguration?.hardwareModel else {
            throw IPSWError.noSupportedConfiguration
        }

        // Check hardware model is supported
        guard hardwareModel.isSupported else {
            throw IPSWError.hardwareModelNotSupported
        }

        // Create bundle directory
        try FileManager.default.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        // Save hardware model
        let hwModelPath = "\(bundlePath)/hardware-model.dat"
        try hardwareModel.dataRepresentation.write(to: URL(fileURLWithPath: hwModelPath))

        // Create machine identifier
        let machineIdentifier = VZMacMachineIdentifier()
        let machineIdPath = "\(bundlePath)/machine-id.dat"
        try machineIdentifier.dataRepresentation.write(to: URL(fileURLWithPath: machineIdPath))

        // Create disk image
        let diskPath = "\(bundlePath)/disk.img"
        let diskSize = Int64(diskSizeGB) * 1024 * 1024 * 1024
        let fd = open(diskPath, O_RDWR | O_CREAT | O_TRUNC, 0o644)
        guard fd >= 0 else {
            throw IPSWError.diskCreationFailed
        }
        let truncResult = ftruncate(fd, off_t(diskSize))
        close(fd)
        guard truncResult == 0 else {
            throw IPSWError.diskCreationFailed
        }

        // Create auxiliary storage
        let auxStoragePath = "\(bundlePath)/aux-storage"
        let auxStorage = try VZMacAuxiliaryStorage(
            creatingStorageAt: URL(fileURLWithPath: auxStoragePath),
            hardwareModel: hardwareModel,
            options: []
        )

        // Configure the VM for installation
        let config = VZVirtualMachineConfiguration()

        let platform = VZMacPlatformConfiguration()
        platform.hardwareModel = hardwareModel
        platform.machineIdentifier = machineIdentifier
        platform.auxiliaryStorage = auxStorage
        config.platform = platform

        config.bootLoader = VZMacOSBootLoader()
        config.cpuCount = max(cpuCount, VZVirtualMachineConfiguration.minimumAllowedCPUCount)
        config.memorySize = max(
            UInt64(memoryGB) * 1024 * 1024 * 1024,
            VZVirtualMachineConfiguration.minimumAllowedMemorySize
        )

        let diskAttachment = try VZDiskImageStorageDeviceAttachment(
            url: URL(fileURLWithPath: diskPath),
            readOnly: false
        )
        config.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: diskAttachment)]

        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
        config.networkDevices = [networkConfig]

        config.graphicsDevices = [{
            let g = VZMacGraphicsDeviceConfiguration()
            g.displays = [VZMacGraphicsDisplayConfiguration(
                widthInPixels: 1920,
                heightInPixels: 1200,
                pixelsPerInch: 144
            )]
            return g
        }()]

        config.keyboards = [VZUSBKeyboardConfiguration()]
        config.pointingDevices = [VZUSBScreenCoordinatePointingDeviceConfiguration()]
        config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        try config.validate()

        let vm = VZVirtualMachine(configuration: config)

        // Perform the install
        logger.info("starting macOS restore from IPSW...")

        let installer = VZMacOSInstaller(virtualMachine: vm, restoringFromImageAt: ipswURL)

        // Observe progress
        let progressObserver = installer.progress.observe(\.fractionCompleted) { progress, _ in
            let pct = Int(progress.fractionCompleted * 100)
            if pct % 10 == 0 {
                print("  install progress: \(pct)%")
            }
        }

        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            installer.install { result in
                switch result {
                case .success:
                    continuation.resume()
                case .failure(let error):
                    continuation.resume(throwing: error)
                }
            }
        }

        progressObserver.invalidate()

        logger.info("macOS installation completed successfully")

        // Mark bundle as installed
        let markerPath = "\(bundlePath)/installed"
        FileManager.default.createFile(atPath: markerPath, contents: nil)
    }

    /// Check if a VM bundle has a completed macOS installation.
    func isInstalled(bundlePath: String) -> Bool {
        return FileManager.default.fileExists(atPath: "\(bundlePath)/installed")
    }
}

enum IPSWError: Error, CustomStringConvertible {
    case downloadFailed(String)
    case noSupportedConfiguration
    case hardwareModelNotSupported
    case diskCreationFailed

    var description: String {
        switch self {
        case .downloadFailed(let msg): return "IPSW download failed: \(msg)"
        case .noSupportedConfiguration: return "no supported configuration in restore image"
        case .hardwareModelNotSupported: return "hardware model not supported on this host"
        case .diskCreationFailed: return "failed to create disk image"
        }
    }
}
