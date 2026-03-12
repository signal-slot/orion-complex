import AppKit
import ArgumentParser
@preconcurrency import Virtualization

@main
struct VMSetupCommand: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "orion-vm-setup",
        abstract: "Open a macOS VM in a window for interactive Setup Assistant"
    )

    @Argument(help: "Path to the VM bundle directory")
    var bundlePath: String

    @Option(name: .long, help: "Number of CPU cores")
    var cpuCount: Int = 4

    @Option(name: .long, help: "Memory in GB")
    var memoryGB: Int = 8

    func run() throws {
        let app = NSApplication.shared
        app.setActivationPolicy(.regular)

        let delegate = VMAppDelegate(
            bundlePath: bundlePath,
            cpuCount: cpuCount,
            memoryGB: memoryGB
        )
        app.delegate = delegate
        app.run()
    }
}

class VMAppDelegate: NSObject, NSApplicationDelegate {
    let bundlePath: String
    let cpuCount: Int
    let memoryGB: Int
    var window: NSWindow?
    var vm: VZVirtualMachine?

    init(bundlePath: String, cpuCount: Int, memoryGB: Int) {
        self.bundlePath = bundlePath
        self.cpuCount = cpuCount
        self.memoryGB = memoryGB
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            let config = try createVMConfig()
            try config.validate()

            let vm = VZVirtualMachine(configuration: config)
            self.vm = vm

            let vmView = VZVirtualMachineView()
            vmView.virtualMachine = vm
            vmView.capturesSystemKeys = true

            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 1280, height: 800),
                styleMask: [.titled, .closable, .resizable, .miniaturizable],
                backing: .buffered,
                defer: false
            )
            window.title = "Orion VM Setup — \(bundlePath.components(separatedBy: "/").last ?? "")"
            window.contentView = vmView
            window.center()
            window.makeKeyAndOrderFront(nil)
            self.window = window

            NSApp.activate(ignoringOtherApps: true)

            vm.start { result in
                switch result {
                case .success:
                    print("VM started — complete Setup Assistant in the window")
                    print("When done, enable Remote Login in System Settings > General > Sharing")
                    print("Then close this window or press Ctrl+C")
                case .failure(let error):
                    print("Failed to start VM: \(error)")
                    NSApp.terminate(nil)
                }
            }
        } catch {
            print("Error: \(error)")
            NSApp.terminate(nil)
        }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return true
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let vm = vm, vm.canRequestStop {
            try? vm.requestStop()
        }
    }

    private func createVMConfig() throws -> VZVirtualMachineConfiguration {
        let config = VZVirtualMachineConfiguration()

        // Platform
        let platform = VZMacPlatformConfiguration()

        let hwModelPath = "\(bundlePath)/hardware-model.dat"
        guard let hwData = FileManager.default.contents(atPath: hwModelPath),
              let hwModel = VZMacHardwareModel(dataRepresentation: hwData) else {
            throw SetupError.message("Cannot load hardware model from \(hwModelPath)")
        }
        platform.hardwareModel = hwModel

        let machineIdPath = "\(bundlePath)/machine-id.dat"
        guard let idData = FileManager.default.contents(atPath: machineIdPath),
              let machineId = VZMacMachineIdentifier(dataRepresentation: idData) else {
            throw SetupError.message("Cannot load machine identifier from \(machineIdPath)")
        }
        platform.machineIdentifier = machineId

        let auxStoragePath = "\(bundlePath)/aux-storage"
        platform.auxiliaryStorage = VZMacAuxiliaryStorage(
            contentsOf: URL(fileURLWithPath: auxStoragePath)
        )

        config.platform = platform
        config.bootLoader = VZMacOSBootLoader()

        // CPU & memory
        config.cpuCount = max(cpuCount, VZVirtualMachineConfiguration.minimumAllowedCPUCount)
        config.memorySize = max(
            UInt64(memoryGB) * 1024 * 1024 * 1024,
            VZVirtualMachineConfiguration.minimumAllowedMemorySize
        )

        // Storage
        let diskPath = "\(bundlePath)/disk.img"
        let diskAttachment = try VZDiskImageStorageDeviceAttachment(
            url: URL(fileURLWithPath: diskPath),
            readOnly: false
        )
        config.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: diskAttachment)]

        // Network
        let networkConfig = VZVirtioNetworkDeviceConfiguration()
        networkConfig.attachment = VZNATNetworkDeviceAttachment()
        config.networkDevices = [networkConfig]

        // Keyboard & mouse
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

        // Entropy
        config.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        // Shared directory
        let sharedDir = VZVirtioFileSystemDeviceConfiguration(tag: "orion-shared")
        let sharedPath = "\(bundlePath)/shared"
        try FileManager.default.createDirectory(atPath: sharedPath, withIntermediateDirectories: true)
        sharedDir.share = VZSingleDirectoryShare(
            directory: VZSharedDirectory(url: URL(fileURLWithPath: sharedPath), readOnly: false)
        )
        config.directorySharingDevices = [sharedDir]

        return config
    }
}

enum SetupError: Error, CustomStringConvertible {
    case message(String)
    var description: String {
        switch self { case .message(let m): return m }
    }
}
