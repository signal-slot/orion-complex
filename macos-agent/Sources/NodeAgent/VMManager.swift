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
        let process: Process?  // nil when adopted from existing process
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

    struct WinInstallOptions: Codable {
        var bypass_tpm: Bool?
        var bypass_secure_boot: Bool?
        var bypass_ram: Bool?
        var bypass_cpu: Bool?
        var language: String?
        var timezone: String?
        var username: String?
        var password: String?
        var auto_login: Bool?
        var auto_partition: Bool?
        var product_key: String?
        var skip_oobe: Bool?
    }

    func createVM(envId: String, cpuCount: Int = 4, memoryGB: Int = 8, guestOS: String = "macos", guestArch: String? = nil, winInstallOptions: WinInstallOptions? = nil) async throws {
        // If guest arch is x86_64 on arm64 host, use QEMU emulation
        if guestArch == "x86_64" {
            // QEMU TCG emulation is slow; use smaller defaults
            let qemuCPUs = min(cpuCount, 2)
            let qemuMemGB = min(memoryGB, 2)
            try await createQEMUVM(envId: envId, cpuCount: qemuCPUs, memoryGB: qemuMemGB, winInstallOptions: winInstallOptions)
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

        // Storage: main disk + cloud-init cidata ISO if present
        let diskAttachment = try VZDiskImageStorageDeviceAttachment(
            url: URL(fileURLWithPath: diskPath),
            readOnly: false
        )
        var linuxStorageDevices: [VZStorageDeviceConfiguration] = [
            VZVirtioBlockDeviceConfiguration(attachment: diskAttachment)
        ]
        let cidataPath = "\(bundlePath)/cidata.iso"
        if FileManager.default.fileExists(atPath: cidataPath) {
            let cidataAttachment = try VZDiskImageStorageDeviceAttachment(
                url: URL(fileURLWithPath: cidataPath),
                readOnly: true
            )
            linuxStorageDevices.append(VZVirtioBlockDeviceConfiguration(attachment: cidataAttachment))
            logger.info("attached cidata ISO for cloud-init")
        }
        config.storageDevices = linuxStorageDevices

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

        // Serial console — write boot output to log file for debugging
        let serialLogPath = "\(bundlePath)/console.log"
        FileManager.default.createFile(atPath: serialLogPath, contents: nil)
        if let serialLog = FileHandle(forWritingAtPath: serialLogPath) {
            let serialPort = VZVirtioConsoleDeviceSerialPortConfiguration()
            serialPort.attachment = VZFileHandleSerialPortAttachment(
                fileHandleForReading: nil,
                fileHandleForWriting: serialLog
            )
            config.serialPorts = [serialPort]
        }

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

    /// Find an available TCP port starting from `base`, skipping ports already in use.
    private func findAvailablePort(base: Int) -> Int {
        var port = base
        while port < 65535 {
            let sock = socket(AF_INET, SOCK_STREAM, 0)
            guard sock >= 0 else { port += 1; continue }
            defer { close(sock) }

            var addr = sockaddr_in()
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = in_port_t(port).bigEndian
            addr.sin_addr.s_addr = INADDR_ANY

            var optval: Int32 = 1
            setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &optval, socklen_t(MemoryLayout<Int32>.size))

            let bindResult = withUnsafePointer(to: &addr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                    bind(sock, sockPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            if bindResult == 0 {
                return port
            }
            port += 1
        }
        return base // fallback
    }

    private func createQEMUVM(envId: String, cpuCount: Int, memoryGB: Int, winInstallOptions: WinInstallOptions? = nil) async throws {
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

        // Check if QEMU is already running for this env (agent restart recovery)
        // Use pgrep to find an existing process referencing this bundle
        let pgrepCheck = Process()
        pgrepCheck.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        pgrepCheck.arguments = ["-f", "\(envId).bundle"]
        let pgrepPipe = Pipe()
        pgrepCheck.standardOutput = pgrepPipe
        pgrepCheck.standardError = FileHandle.nullDevice
        try? pgrepCheck.run()
        pgrepCheck.waitUntilExit()
        let pgrepOutput = String(data: pgrepPipe.fileHandleForReading.availableData, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if pgrepCheck.terminationStatus == 0 && !pgrepOutput.isEmpty {
            // QEMU process exists — adopt it. Parse ports from process args.
            var adoptSSH = 12022
            var adoptVNC = 15950
            let psProc = Process()
            psProc.executableURL = URL(fileURLWithPath: "/bin/ps")
            psProc.arguments = ["-p", pgrepOutput.components(separatedBy: "\n").first ?? "", "-o", "args="]
            let psPipe = Pipe()
            psProc.standardOutput = psPipe
            psProc.standardError = FileHandle.nullDevice
            try? psProc.run()
            psProc.waitUntilExit()
            let psArgs = String(data: psPipe.fileHandleForReading.availableData, encoding: .utf8) ?? ""
            // Parse hostfwd=tcp::NNNNN-:22
            if let range = psArgs.range(of: "hostfwd=tcp::(\\d+)-:22", options: .regularExpression) {
                let match = psArgs[range]
                if let portRange = match.range(of: "\\d+", options: .regularExpression) {
                    adoptSSH = Int(match[portRange]) ?? adoptSSH
                }
            }
            // Parse -vnc 127.0.0.1:NNNNN (offset from 5900)
            if let range = psArgs.range(of: "-vnc 127\\.0\\.0\\.1:(\\d+)", options: .regularExpression) {
                let match = psArgs[range]
                if let portRange = match.range(of: "\\d+$", options: .regularExpression) {
                    let vncOffset = Int(match[portRange]) ?? 10050
                    adoptVNC = 5900 + vncOffset
                }
            }
            qemuVMs[envId] = RunningQEMUVM(
                process: nil,
                bundlePath: bundlePath,
                monitorSocketPath: monitorSocket,
                sshHostPort: adoptSSH,
                vncHostPort: adoptVNC
            )
            logger.info("adopted existing QEMU process for \(envId) (pid \(pgrepOutput), SSH:\(adoptSSH), VNC:\(adoptVNC))")
            return
        }

        // Allocate available host ports for SSH and VNC forwarding
        let sshPort = findAvailablePort(base: 12022)
        let vncPort = findAvailablePort(base: 15950)

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

        // ISO installs use SeaBIOS (OVMF times out under TCG emulation) + IDE.
        // Template-based VMs use OVMF (UEFI) + virtio for better performance.
        let installISOPath = "\(bundlePath)/install.iso"
        let hasISO = FileManager.default.fileExists(atPath: installISOPath)

        var args = [
            "-machine", "q35",
            "-accel", "tcg",
            "-cpu", "max",
            "-smp", "\(cpuCount)",
            "-m", "\(memoryMB)M",
        ]

        if hasISO {
            // SeaBIOS mode for ISO install
            args += [
                "-drive", "file=\(diskPath),format=qcow2,if=ide",
                "-drive", "file=\(installISOPath),media=cdrom,index=1",
                "-boot", "d",
            ]
        } else {
            // UEFI mode for template-based VMs
            args += [
                "-drive", "if=pflash,format=raw,readonly=on,file=\(firmwarePath)",
                "-drive", "if=pflash,format=raw,file=\(efiVarsPath)",
                "-drive", "file=\(diskPath),format=qcow2,if=virtio",
            ]
        }

        args += [
            "-netdev", "user,id=net0,hostfwd=tcp::\(sshPort)-:22",
            "-device", "\(hasISO ? "e1000" : "virtio-net-pci"),netdev=net0",
            "-display", "none",
            "-vnc", "127.0.0.1:\(vncPort - 5900)",
            "-device", "virtio-gpu-pci",
            "-serial", "mon:stdio",
            "-monitor", "unix:\(monitorSocket),server,nowait",
            "-device", "virtio-rng-pci",
        ]

        // Attach cloud-init ISO if present
        if FileManager.default.fileExists(atPath: ciISOPath) {
            args += ["-drive", "file=\(ciISOPath),format=raw,if=virtio,readonly=on"]
        }

        // Generate autounattend ISO for Windows install automation
        if let opts = winInstallOptions, hasISO {
            let xml = generateAutounattendXML(opts)
            if !xml.isEmpty {
                let unattendDir = "\(bundlePath)/unattend"
                try FileManager.default.createDirectory(atPath: unattendDir, withIntermediateDirectories: true)
                try xml.write(toFile: "\(unattendDir)/autounattend.xml", atomically: true, encoding: .utf8)

                let unattendISO = "\(bundlePath)/autounattend.iso"
                if !FileManager.default.fileExists(atPath: unattendISO) {
                    let hdiutil = Process()
                    hdiutil.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
                    hdiutil.arguments = ["makehybrid", "-o", unattendISO,
                                         "-default-volume-name", "OEMDRV",
                                         "-iso", "-joliet",
                                         unattendDir]
                    let errPipe = Pipe()
                    hdiutil.standardOutput = FileHandle.nullDevice
                    hdiutil.standardError = errPipe
                    try hdiutil.run()
                    hdiutil.waitUntilExit()
                    if hdiutil.terminationStatus != 0 {
                        let errMsg = String(data: errPipe.fileHandleForReading.availableData, encoding: .utf8) ?? ""
                        logger.warning("[\(envId)] hdiutil autounattend failed: \(errMsg)")
                    }
                }

                if FileManager.default.fileExists(atPath: unattendISO) {
                    // Attach as additional CD-ROM — Windows Setup searches all
                    // removable media (including CD/DVD drives) for autounattend.xml
                    args += ["-drive", "file=\(unattendISO),media=cdrom,index=2"]
                    logger.info("[\(envId)] attached autounattend ISO")
                }
            }
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
    private func generateAutounattendXML(_ opts: WinInstallOptions) -> String {
        let arch = "amd64"
        let token = "31bf3856ad364e35"
        let lang = opts.language ?? "en-US"

        // -- windowsPE pass --
        var runSyncCmds = ""
        var order = 1
        let bypasses: [(Bool?, String)] = [
            (opts.bypass_tpm, "BypassTPMCheck"),
            (opts.bypass_secure_boot, "BypassSecureBootCheck"),
            (opts.bypass_ram, "BypassRAMCheck"),
            (opts.bypass_cpu, "BypassCPUCheck"),
        ]
        for (flag, name) in bypasses {
            if flag == true {
                runSyncCmds += """
                    <RunSynchronousCommand wcm:action="add">
                      <Order>\(order)</Order>
                      <Path>reg add HKLM\\SYSTEM\\Setup\\LabConfig /v \(name) /t REG_DWORD /d 1 /f</Path>
                    </RunSynchronousCommand>\n
                """
                order += 1
            }
        }

        var setupComponent = ""
        if !runSyncCmds.isEmpty {
            setupComponent += "      <RunSynchronous>\n\(runSyncCmds)      </RunSynchronous>\n"
        }
        if opts.auto_partition == true {
            setupComponent += """
                  <DiskConfiguration>
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
                  </ImageInstall>\n
            """
        }
        if let key = opts.product_key, !key.isEmpty {
            setupComponent += """
                  <UserData>
                    <ProductKey>
                      <Key>\(key)</Key>
                    </ProductKey>
                    <AcceptEula>true</AcceptEula>
                  </UserData>\n
            """
        }

        var windowsPE = ""
        if !setupComponent.isEmpty {
            windowsPE += """
                <component name="Microsoft-Windows-Setup"
                           processorArchitecture="\(arch)"
                           publicKeyToken="\(token)"
                           language="neutral"
                           versionScope="nonSxS">
            \(setupComponent)    </component>\n
            """
        }
        // Language for WinPE
        windowsPE += """
              <component name="Microsoft-Windows-International-Core-WinPE"
                         processorArchitecture="\(arch)"
                         publicKeyToken="\(token)"
                         language="neutral"
                         versionScope="nonSxS">
                <SetupUILanguage>
                  <UILanguage>\(lang)</UILanguage>
                </SetupUILanguage>
                <InputLocale>\(lang)</InputLocale>
                <SystemLocale>\(lang)</SystemLocale>
                <UILanguage>\(lang)</UILanguage>
                <UserLocale>\(lang)</UserLocale>
              </component>\n
        """

        // -- specialize pass --
        var specialize = ""
        if let tz = opts.timezone, !tz.isEmpty {
            specialize = """
              <component name="Microsoft-Windows-Shell-Setup"
                         processorArchitecture="\(arch)"
                         publicKeyToken="\(token)"
                         language="neutral"
                         versionScope="nonSxS">
                <TimeZone>\(tz)</TimeZone>
              </component>\n
            """
        }

        // -- oobeSystem pass --
        var oobeInner = ""

        // Always hide Microsoft account / online screens
        oobeInner += """
                <OOBE>
                  <HideEULAPage>true</HideEULAPage>
                  <HideWirelessSetupInOOBE>true</HideWirelessSetupInOOBE>
                  <ProtectYourPC>3</ProtectYourPC>
                  <HideOnlineAccountScreens>true</HideOnlineAccountScreens>
        """
        if opts.skip_oobe == true {
            oobeInner += """
                          <SkipMachineOOBE>true</SkipMachineOOBE>
                          <SkipUserOOBE>true</SkipUserOOBE>
            """
        }
        oobeInner += "        </OOBE>\n"

        // Local account (default to "user" if not specified)
        let username = (opts.username?.isEmpty == false) ? opts.username! : "user"
        let password = opts.password ?? ""
        oobeInner += """
                <UserAccounts>
                  <LocalAccounts>
                    <LocalAccount wcm:action="add">
                      <Name>\(username)</Name>
                      <Group>Administrators</Group>
                      <Password>
                        <Value>\(password)</Value>
                        <PlainText>true</PlainText>
                      </Password>
                    </LocalAccount>
                  </LocalAccounts>
                </UserAccounts>\n
        """
        if opts.auto_login == true {
            oobeInner += """
                    <AutoLogon>
                      <Enabled>true</Enabled>
                      <Username>\(username)</Username>
                      <Password>
                        <Value>\(password)</Value>
                        <PlainText>true</PlainText>
                      </Password>
                      <LogonCount>999</LogonCount>
                    </AutoLogon>\n
            """
        }

        // FirstLogonCommands: install & enable OpenSSH Server, open firewall
        oobeInner += """
                <FirstLogonCommands>
                  <SynchronousCommand wcm:action="add">
                    <Order>1</Order>
                    <CommandLine>powershell -NoProfile -Command "Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0"</CommandLine>
                    <Description>Install OpenSSH Server</Description>
                  </SynchronousCommand>
                  <SynchronousCommand wcm:action="add">
                    <Order>2</Order>
                    <CommandLine>powershell -NoProfile -Command "Start-Service sshd; Set-Service -Name sshd -StartupType Automatic"</CommandLine>
                    <Description>Start and enable OpenSSH Server</Description>
                  </SynchronousCommand>
                  <SynchronousCommand wcm:action="add">
                    <Order>3</Order>
                    <CommandLine>powershell -NoProfile -Command "New-NetFirewallRule -Name sshd -DisplayName 'OpenSSH Server' -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22"</CommandLine>
                    <Description>Open SSH firewall port</Description>
                  </SynchronousCommand>
                </FirstLogonCommands>\n
        """

        var oobeSystem = ""
        if !oobeInner.isEmpty {
            oobeSystem = """
              <component name="Microsoft-Windows-Shell-Setup"
                         processorArchitecture="\(arch)"
                         publicKeyToken="\(token)"
                         language="neutral"
                         versionScope="nonSxS">
            \(oobeInner)    </component>\n
            """
        }
        // Language in oobeSystem
        oobeSystem += """
              <component name="Microsoft-Windows-International-Core"
                         processorArchitecture="\(arch)"
                         publicKeyToken="\(token)"
                         language="neutral"
                         versionScope="nonSxS">
                <InputLocale>\(lang)</InputLocale>
                <SystemLocale>\(lang)</SystemLocale>
                <UILanguage>\(lang)</UILanguage>
                <UserLocale>\(lang)</UserLocale>
              </component>\n
        """

        var xml = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n"
        xml += "<unattend xmlns=\"urn:schemas-microsoft-com:unattend\"\n"
        xml += "          xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\n"
        xml += "  <settings pass=\"windowsPE\">\n\(windowsPE)  </settings>\n"
        if !specialize.isEmpty {
            xml += "  <settings pass=\"specialize\">\n\(specialize)  </settings>\n"
        }
        xml += "  <settings pass=\"oobeSystem\">\n\(oobeSystem)  </settings>\n"
        xml += "</unattend>\n"
        return xml
    }

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
            if let proc = qemu.process, proc.isRunning {
                proc.terminate()
                proc.waitUntilExit()
            }
            // Also kill any orphaned QEMU process for this env
            let pkill = Process()
            pkill.executableURL = URL(fileURLWithPath: "/usr/bin/pkill")
            pkill.arguments = ["-f", "\(envId).bundle"]
            try? pkill.run()
            pkill.waitUntilExit()
            try await Task.sleep(nanoseconds: 1_000_000_000)
            try FileManager.default.removeItem(atPath: qemu.bundlePath)
            logger.info("QEMU VM destroyed for environment \(envId)")
            return
        }

        guard let running = vms.removeValue(forKey: envId) else {
            throw VMError.notFound(envId)
        }

        logger.info("destroying VM for environment \(envId)")

        // Entire VM stop with a hard 15s timeout
        do {
            try await withThrowingTaskGroup(of: Void.self) { group in
                group.addTask {
                    // Try graceful stop first
                    if running.vm.canRequestStop {
                        try? running.vm.requestStop()
                        for _ in 0..<10 {
                            try await Task.sleep(nanoseconds: 500_000_000)
                            if running.vm.state == .stopped { return }
                        }
                    }
                    // Force stop
                    if running.vm.state != .stopped {
                        try await self.stopVMOnMain(running.vm)
                    }
                }
                group.addTask {
                    try await Task.sleep(nanoseconds: 15_000_000_000)
                    throw VMError.notFound("VM stop timeout")
                }
                try await group.next()
                group.cancelAll()
            }
        } catch {
            logger.warning("VM stop timed out for \(envId), proceeding with cleanup anyway")
        }

        // Remove the bundle directory regardless of stop result
        try? FileManager.default.removeItem(atPath: running.bundlePath)
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
            if let proc = qemu.process {
                return proc.isRunning ? .running : .stopped
            }
            // Adopted process — check if QEMU is still running via pgrep
            let pgrep = Process()
            pgrep.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
            pgrep.arguments = ["-f", "\(envId).bundle"]
            pgrep.standardOutput = FileHandle.nullDevice
            pgrep.standardError = FileHandle.nullDevice
            try? pgrep.run()
            pgrep.waitUntilExit()
            return pgrep.terminationStatus == 0 ? .running : .stopped
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
