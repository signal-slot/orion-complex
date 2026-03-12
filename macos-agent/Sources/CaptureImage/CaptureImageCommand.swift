import Foundation
@preconcurrency import Virtualization
import ArgumentParser

@main
struct CaptureImageCommand: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "orion-capture-image",
        abstract: "Capture a VM bundle as a golden image template for fast cloning"
    )

    @Argument(help: "Path to the source VM bundle (must have completed Setup Assistant)")
    var bundlePath: String

    @Option(name: .long, help: "Image ID for the template")
    var imageId: String

    @Option(name: .long, help: "Bundle store path (default: ~/.orion/bundles)")
    var bundleStore: String?

    func run() throws {
        let effectiveBundleStore = bundleStore
            ?? ProcessInfo.processInfo.environment["ORION_BUNDLE_STORE"]
            ?? NSHomeDirectory() + "/.orion/bundles"

        let imagesPath = "\(effectiveBundleStore)/images/\(imageId)"
        let fm = FileManager.default

        // Validate source
        let requiredFiles = ["disk.img", "hardware-model.dat", "aux-storage", "installed"]
        for file in requiredFiles {
            guard fm.fileExists(atPath: "\(bundlePath)/\(file)") else {
                print("Error: source bundle missing \(file)")
                print("Make sure the VM has been set up (IPSW installed, Setup Assistant completed)")
                throw ExitCode.failure
            }
        }

        // Check destination doesn't already exist
        if fm.fileExists(atPath: imagesPath) {
            print("Error: template already exists at \(imagesPath)")
            print("Remove it first if you want to replace it")
            throw ExitCode.failure
        }

        try fm.createDirectory(atPath: imagesPath, withIntermediateDirectories: true)

        print("Capturing golden image from \(bundlePath)...")

        // Copy files (APFS clone makes this fast)
        print("  copying disk.img...")
        try fm.copyItem(
            atPath: "\(bundlePath)/disk.img",
            toPath: "\(imagesPath)/disk.img"
        )

        print("  copying aux-storage...")
        try fm.copyItem(
            atPath: "\(bundlePath)/aux-storage",
            toPath: "\(imagesPath)/aux-storage"
        )

        print("  copying hardware-model.dat...")
        try fm.copyItem(
            atPath: "\(bundlePath)/hardware-model.dat",
            toPath: "\(imagesPath)/hardware-model.dat"
        )

        // Write metadata
        let metadata: [String: String] = [
            "source_bundle": bundlePath,
            "captured_at": ISO8601DateFormatter().string(from: Date()),
            "image_id": imageId,
        ]
        let data = try JSONSerialization.data(
            withJSONObject: metadata,
            options: [.prettyPrinted, .sortedKeys]
        )
        try data.write(to: URL(fileURLWithPath: "\(imagesPath)/metadata.json"))

        print("")
        print("Golden image captured successfully!")
        print("  Template: \(imagesPath)")
        print("  Image ID: \(imageId)")
        print("")
        print("To use: create environments with image_id=\"\(imageId)\"")
        print("The node agent will clone this template instead of IPSW restore (~seconds vs ~minutes)")
    }
}
