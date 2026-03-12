// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "OrionMacOSAgent",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "orion-node-agent", targets: ["NodeAgent"]),
        .executable(name: "orion-guest-agent", targets: ["GuestAgent"]),
        .executable(name: "orion-vm-setup", targets: ["VMSetup"]),
        .executable(name: "orion-capture-image", targets: ["CaptureImage"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-argument-parser", from: "1.3.0"),
        .package(url: "https://github.com/swift-server/async-http-client", from: "1.21.0"),
        .package(url: "https://github.com/apple/swift-log", from: "1.5.0"),
    ],
    targets: [
        .executableTarget(
            name: "NodeAgent",
            dependencies: [
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
                .product(name: "AsyncHTTPClient", package: "async-http-client"),
                .product(name: "Logging", package: "swift-log"),
            ]
        ),
        .executableTarget(
            name: "GuestAgent",
            dependencies: [
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
                .product(name: "Logging", package: "swift-log"),
            ]
        ),
        .executableTarget(
            name: "VMSetup",
            dependencies: [
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
            ]
        ),
        .executableTarget(
            name: "CaptureImage",
            dependencies: [
                .product(name: "ArgumentParser", package: "swift-argument-parser"),
            ]
        ),
    ]
)
