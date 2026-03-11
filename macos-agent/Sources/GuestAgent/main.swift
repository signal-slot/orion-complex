import Foundation
import ArgumentParser
import Logging

/// Guest agent that runs inside a macOS VM.
/// Communicates with the host via a shared directory (Virtio filesystem).
///
/// Responsibilities:
/// - Create/sync user accounts from the control plane
/// - Install SSH authorized_keys
/// - Report health status
/// - Handle graceful shutdown requests
@main
struct GuestAgentCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "orion-guest-agent",
        abstract: "Orion guest agent — runs inside macOS VMs for provisioning"
    )

    @Option(name: .long, help: "Path to the shared directory mounted from the host")
    var sharedDir: String = "/Volumes/orion-shared"

    @Option(name: .long, help: "Poll interval in seconds")
    var pollInterval: Int = 5

    func run() async throws {
        LoggingSystem.bootstrap { label in
            var handler = StreamLogHandler.standardOutput(label: label)
            handler.logLevel = .info
            return handler
        }
        let logger = Logger(label: "orion.guest-agent")

        logger.info("starting orion-guest-agent")
        logger.info("  shared directory: \(sharedDir)")

        let provisioner = Provisioner(sharedDir: sharedDir, logger: logger)

        // Write initial status
        provisioner.writeStatus(.init(state: "running", message: "guest agent started"))

        // Main loop: watch shared directory for provisioning commands
        while !Task.isCancelled {
            do {
                try await provisioner.poll()
            } catch {
                logger.error("provisioning error: \(error)")
            }

            try await Task.sleep(nanoseconds: UInt64(pollInterval) * 1_000_000_000)
        }
    }
}

// MARK: - Provisioner

struct GuestStatus: Codable {
    let state: String
    let message: String
    let timestamp: Int64

    init(state: String, message: String) {
        self.state = state
        self.message = message
        self.timestamp = Int64(Date().timeIntervalSince1970)
    }
}

struct ProvisioningRequest: Codable {
    let action: String // "sync_ssh_keys", "create_user", "shutdown"
    let username: String?
    let ssh_keys: [String]?
}

class Provisioner {
    private let sharedDir: String
    private let logger: Logger

    init(sharedDir: String, logger: Logger) {
        self.sharedDir = sharedDir
        self.logger = logger
    }

    func writeStatus(_ status: GuestStatus) {
        let statusPath = "\(sharedDir)/guest-status.json"
        do {
            let data = try JSONEncoder().encode(status)
            try data.write(to: URL(fileURLWithPath: statusPath))
        } catch {
            logger.error("failed to write status: \(error)")
        }
    }

    func poll() async throws {
        let requestPath = "\(sharedDir)/provision-request.json"

        guard FileManager.default.fileExists(atPath: requestPath) else {
            return
        }

        let data = try Data(contentsOf: URL(fileURLWithPath: requestPath))
        let request = try JSONDecoder().decode(ProvisioningRequest.self, from: data)

        // Remove the request file to acknowledge it
        try FileManager.default.removeItem(atPath: requestPath)

        logger.info("processing provisioning request: \(request.action)")

        switch request.action {
        case "sync_ssh_keys":
            if let username = request.username, let keys = request.ssh_keys {
                try syncSSHKeys(username: username, keys: keys)
                writeStatus(.init(state: "running", message: "SSH keys synced for \(username)"))
            }

        case "create_user":
            if let username = request.username {
                try createUser(username: username)
                if let keys = request.ssh_keys {
                    try syncSSHKeys(username: username, keys: keys)
                }
                writeStatus(.init(state: "running", message: "user \(username) created"))
            }

        case "shutdown":
            logger.info("shutdown requested")
            writeStatus(.init(state: "shutting_down", message: "shutdown requested"))
            // Give time for status to be read
            try await Task.sleep(nanoseconds: 2_000_000_000)
            // Trigger macOS shutdown
            let process = Process()
            process.executableURL = URL(fileURLWithPath: "/sbin/shutdown")
            process.arguments = ["-h", "now"]
            try process.run()

        default:
            logger.warning("unknown action: \(request.action)")
        }
    }

    // MARK: - User management

    private func createUser(username: String) throws {
        logger.info("creating user: \(username)")

        // Use sysadminctl to create the user on macOS
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/sysadminctl")
        process.arguments = ["-addUser", username, "-password", "", "-admin"]

        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe

        try process.run()
        process.waitUntilExit()

        if process.terminationStatus != 0 {
            let output = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            logger.error("failed to create user \(username): \(output)")
        } else {
            logger.info("user \(username) created successfully")
        }
    }

    private func syncSSHKeys(username: String, keys: [String]) throws {
        logger.info("syncing \(keys.count) SSH key(s) for user \(username)")

        // Resolve home directory for the user
        let homeDir: String
        if username == NSUserName() {
            homeDir = NSHomeDirectory()
        } else {
            homeDir = "/Users/\(username)"
        }

        let sshDir = "\(homeDir)/.ssh"
        let authorizedKeysPath = "\(sshDir)/authorized_keys"

        // Create .ssh directory if needed
        try FileManager.default.createDirectory(
            atPath: sshDir,
            withIntermediateDirectories: true
        )

        // Write authorized_keys
        let content = keys.joined(separator: "\n") + "\n"
        try content.write(toFile: authorizedKeysPath, atomically: true, encoding: .utf8)

        // Set correct permissions
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o700],
            ofItemAtPath: sshDir
        )
        try FileManager.default.setAttributes(
            [.posixPermissions: 0o600],
            ofItemAtPath: authorizedKeysPath
        )

        logger.info("SSH keys synced for \(username)")
    }
}
