import Foundation
@preconcurrency import Virtualization
import Logging

/// Manages golden image templates for fast VM cloning.
final class TemplateManager {
    private let imagesPath: String
    private let logger = Logger(label: "orion.node-agent.template")

    init(bundleStorePath: String) {
        self.imagesPath = "\(bundleStorePath)/images"
    }

    /// Check if a local golden image template exists for the given image ID (defaults to macOS).
    func hasTemplate(imageId: String) -> Bool {
        return hasTemplate(imageId: imageId, guestOS: "macos")
    }

    /// Check if a local golden image template exists for the given image ID and guest OS.
    func hasTemplate(imageId: String, guestOS: String) -> Bool {
        let templatePath = "\(imagesPath)/\(imageId)"
        let fm = FileManager.default
        guard fm.fileExists(atPath: "\(templatePath)/disk.img") else { return false }
        if guestOS == "macos" {
            return fm.fileExists(atPath: "\(templatePath)/hardware-model.dat")
                && fm.fileExists(atPath: "\(templatePath)/aux-storage")
        }
        return true
    }

    /// Clone a golden image template into a new VM bundle directory.
    /// Creates a fresh machine-id.dat for uniqueness.
    func cloneTemplate(imageId: String, toBundlePath bundlePath: String) throws {
        let templatePath = "\(imagesPath)/\(imageId)"
        let fm = FileManager.default

        try fm.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        // Copy disk image (APFS uses clonefile — nearly instant for sparse files)
        try fm.copyItem(atPath: "\(templatePath)/disk.img", toPath: "\(bundlePath)/disk.img")

        // Copy auxiliary storage (NVRAM)
        try fm.copyItem(atPath: "\(templatePath)/aux-storage", toPath: "\(bundlePath)/aux-storage")

        // Copy hardware model (must match the installed macOS)
        try fm.copyItem(
            atPath: "\(templatePath)/hardware-model.dat",
            toPath: "\(bundlePath)/hardware-model.dat"
        )

        // Generate a FRESH machine identifier (critical for VM uniqueness)
        let machineId = VZMacMachineIdentifier()
        try machineId.dataRepresentation.write(
            to: URL(fileURLWithPath: "\(bundlePath)/machine-id.dat")
        )

        // Create shared directory
        try fm.createDirectory(
            atPath: "\(bundlePath)/shared",
            withIntermediateDirectories: true
        )

        // Mark as installed so PollCycle skips IPSW restore
        fm.createFile(atPath: "\(bundlePath)/installed", contents: nil)

        logger.info("cloned template \(imageId) to \(bundlePath)")
    }

    /// Clone a Linux golden image template into a new VM bundle directory.
    /// Creates a fresh EFI variable store instead of copying macOS-specific files.
    func cloneLinuxTemplate(imageId: String, toBundlePath bundlePath: String) throws {
        let templatePath = "\(imagesPath)/\(imageId)"
        let fm = FileManager.default

        try fm.createDirectory(atPath: bundlePath, withIntermediateDirectories: true)

        // Copy disk image (APFS uses clonefile — nearly instant for sparse files)
        try fm.copyItem(atPath: "\(templatePath)/disk.img", toPath: "\(bundlePath)/disk.img")

        // Create shared directory
        try fm.createDirectory(
            atPath: "\(bundlePath)/shared",
            withIntermediateDirectories: true
        )

        // Create a fresh EFI variable store
        let efiPath = "\(bundlePath)/efi-variable-store"
        let _ = try VZEFIVariableStore(creatingVariableStoreAt: URL(fileURLWithPath: efiPath))

        // Mark as installed so PollCycle skips restore
        fm.createFile(atPath: "\(bundlePath)/installed", contents: nil)

        logger.info("cloned Linux template \(imageId) to \(bundlePath)")
    }

    /// Provision a cloned VM disk with SSH authorized_keys and hostname.
    /// Mounts the disk image, writes keys, sets hostname, then unmounts.
    func provisionDisk(bundlePath: String, authorizedKeys: [String], hostname: String?) throws {
        let diskPath = "\(bundlePath)/disk.img"
        guard FileManager.default.fileExists(atPath: diskPath) else {
            throw TemplateError.missingDisk(bundlePath)
        }

        // Attach disk without mounting
        let attachResult = try shellOut("hdiutil", "attach", "-nomount", diskPath)
        guard let diskDevice = attachResult.components(separatedBy: "\n")
            .first?.components(separatedBy: "\t").first?.trimmingCharacters(in: .whitespaces) else {
            throw TemplateError.mountFailed("could not parse hdiutil output")
        }
        logger.info("attached disk at \(diskDevice)")

        defer {
            // Always detach on exit
            let _ = try? shellOut("hdiutil", "detach", diskDevice)
            logger.info("detached \(diskDevice)")
        }

        // Find the APFS data volume (Role = Data)
        let listOutput = try shellOut("diskutil", "apfs", "list", "-plist")
        guard let dataVolume = findDataVolume(in: listOutput, forDisk: diskDevice) else {
            throw TemplateError.mountFailed("could not find APFS data volume")
        }
        logger.info("found data volume: \(dataVolume)")

        // Mount with noowners
        let _ = try shellOut("diskutil", "mount", "-mountOptions", "noowners", dataVolume)

        // Find the mount point
        let mountInfo = try shellOut("diskutil", "info", "-plist", dataVolume)
        guard let mountPoint = extractMountPoint(from: mountInfo) else {
            throw TemplateError.mountFailed("could not determine mount point for \(dataVolume)")
        }
        logger.info("mounted at \(mountPoint)")

        defer {
            let _ = try? shellOut("diskutil", "unmount", dataVolume)
        }

        // Write authorized_keys
        if !authorizedKeys.isEmpty {
            let sshDir = "\(mountPoint)/private/var/root/.ssh"
            try FileManager.default.createDirectory(atPath: sshDir, withIntermediateDirectories: true)
            let keysContent = authorizedKeys.joined(separator: "\n") + "\n"
            try keysContent.write(
                toFile: "\(sshDir)/authorized_keys",
                atomically: true,
                encoding: .utf8
            )
            logger.info("wrote \(authorizedKeys.count) authorized key(s)")
        }

        // Set hostname
        if let hostname = hostname {
            let prefsDir = "\(mountPoint)/Library/Preferences/SystemConfiguration"
            try FileManager.default.createDirectory(atPath: prefsDir, withIntermediateDirectories: true)

            let prefsPlist = """
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
                <key>System</key>
                <dict>
                    <key>Network</key>
                    <dict>
                        <key>HostNames</key>
                        <dict>
                            <key>LocalHostName</key>
                            <string>\(hostname)</string>
                        </dict>
                    </dict>
                    <key>System</key>
                    <dict>
                        <key>ComputerName</key>
                        <string>\(hostname)</string>
                        <key>HostName</key>
                        <string>\(hostname).local</string>
                    </dict>
                </dict>
            </dict>
            </plist>
            """
            try prefsPlist.write(
                toFile: "\(prefsDir)/preferences.plist",
                atomically: true,
                encoding: .utf8
            )
            logger.info("hostname set to \(hostname)")
        }
    }

    // MARK: - Shell helpers

    @discardableResult
    private func shellOut(_ args: String...) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        process.waitUntilExit()
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        let output = String(data: data, encoding: .utf8) ?? ""
        if process.terminationStatus != 0 {
            throw TemplateError.shellError("\(args.joined(separator: " ")) failed: \(output)")
        }
        return output
    }

    private func findDataVolume(in plistOutput: String, forDisk diskDevice: String) -> String? {
        // Parse diskutil apfs list -plist output to find the Data volume
        // The disk device is like /dev/disk12, and we need to find the container
        // that has a physical store on disk12s2, then find the Data role volume
        guard let data = plistOutput.data(using: .utf8),
              let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              let containers = plist["Containers"] as? [[String: Any]] else {
            return nil
        }

        let diskName = diskDevice.replacingOccurrences(of: "/dev/", with: "")

        for container in containers {
            // Check if this container's physical stores reference our disk
            guard let stores = container["PhysicalStores"] as? [[String: Any]] else { continue }
            let matchesDisk = stores.contains { store in
                guard let storeId = store["DeviceIdentifier"] as? String else { return false }
                return storeId.hasPrefix(diskName)
            }
            guard matchesDisk else { continue }

            // Find the Data volume
            guard let volumes = container["Volumes"] as? [[String: Any]] else { continue }
            for volume in volumes {
                guard let roles = volume["Roles"] as? [String],
                      roles.contains("Data"),
                      let deviceId = volume["DeviceIdentifier"] as? String else { continue }
                return "/dev/\(deviceId)"
            }
        }
        return nil
    }

    private func extractMountPoint(from plistOutput: String) -> String? {
        guard let data = plistOutput.data(using: .utf8),
              let plist = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              let mountPoint = plist["MountPoint"] as? String else {
            return nil
        }
        return mountPoint
    }

    /// Capture a VM bundle as a golden image template.
    func captureTemplate(fromBundlePath bundlePath: String, imageId: String, guestOS: String = "macos") throws {
        let templatePath = "\(imagesPath)/\(imageId)"
        let fm = FileManager.default

        // Verify source bundle
        guard fm.fileExists(atPath: "\(bundlePath)/disk.img"),
              fm.fileExists(atPath: "\(bundlePath)/installed") else {
            throw TemplateError.incompletBundle(bundlePath)
        }

        if guestOS == "macos" {
            guard fm.fileExists(atPath: "\(bundlePath)/hardware-model.dat"),
                  fm.fileExists(atPath: "\(bundlePath)/aux-storage") else {
                throw TemplateError.incompletBundle(bundlePath)
            }
        }

        try fm.createDirectory(atPath: templatePath, withIntermediateDirectories: true)

        try fm.copyItem(atPath: "\(bundlePath)/disk.img", toPath: "\(templatePath)/disk.img")

        if guestOS == "macos" {
            try fm.copyItem(atPath: "\(bundlePath)/aux-storage", toPath: "\(templatePath)/aux-storage")
            try fm.copyItem(
                atPath: "\(bundlePath)/hardware-model.dat",
                toPath: "\(templatePath)/hardware-model.dat"
            )
        }

        // Write metadata
        let metadata: [String: String] = [
            "source_bundle": bundlePath,
            "captured_at": ISO8601DateFormatter().string(from: Date()),
            "guest_os": guestOS,
        ]
        let data = try JSONEncoder().encode(metadata)
        try data.write(to: URL(fileURLWithPath: "\(templatePath)/metadata.json"))

        logger.info("captured template \(imageId) from \(bundlePath) (guest_os: \(guestOS))")
    }
}

enum TemplateError: Error, CustomStringConvertible {
    case incompletBundle(String)
    case missingDisk(String)
    case mountFailed(String)
    case shellError(String)

    var description: String {
        switch self {
        case .incompletBundle(let path):
            return "bundle at \(path) is incomplete (missing disk.img, hardware-model.dat, aux-storage, or installed marker)"
        case .missingDisk(let path):
            return "disk.img not found in \(path)"
        case .mountFailed(let msg):
            return "failed to mount disk: \(msg)"
        case .shellError(let msg):
            return msg
        }
    }
}
