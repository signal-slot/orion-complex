import Foundation
import ArgumentParser
import Logging

@main
struct NodeAgentCommand: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "orion-node-agent",
        abstract: "Orion macOS node agent — manages macOS VMs via Virtualization.framework"
    )

    @Option(name: .long, help: "Control plane URL")
    var controlPlane: String?

    @Option(name: .long, help: "Node name")
    var nodeName: String?

    @Option(name: .long, help: "API token for authenticating with the control plane")
    var token: String?

    @Option(name: .long, help: "Path to VM bundle storage")
    var bundleStore: String?

    @Option(name: .long, help: "Poll interval in seconds")
    var pollInterval: Int?

    @Flag(name: .long, help: "Download the latest macOS IPSW on startup")
    var downloadIpsw: Bool = false

    func run() async throws {
        LoggingSystem.bootstrap { label in
            var handler = StreamLogHandler.standardOutput(label: label)
            handler.logLevel = .info
            return handler
        }
        let logger = Logger(label: "orion.node-agent")

        let config = AgentConfig.fromEnvironment()
        let effectiveControlPlane = controlPlane ?? config.controlPlaneURL.absoluteString
        let effectiveNodeName = nodeName ?? config.nodeName
        let effectiveToken = token ?? config.apiToken
        let effectiveBundleStore = bundleStore ?? config.bundleStorePath
        let effectivePollInterval = pollInterval ?? config.pollIntervalSeconds

        logger.info("starting orion-node-agent")
        logger.info("  control plane: \(effectiveControlPlane)")
        logger.info("  node name: \(effectiveNodeName)")
        logger.info("  bundle store: \(effectiveBundleStore)")

        let api = APIClient(
            baseURL: URL(string: effectiveControlPlane)!,
            token: effectiveToken
        )

        let vmManager = VMManager(bundleStorePath: effectiveBundleStore)
        let ipswRestore = IPSWRestore(bundleStorePath: effectiveBundleStore)

        // Download IPSW if requested
        if downloadIpsw {
            logger.info("downloading latest macOS IPSW...")
            do {
                let (url, _) = try await ipswRestore.downloadLatestIPSW()
                logger.info("IPSW ready at \(url.path)")
            } catch {
                logger.error("failed to download IPSW: \(error)")
            }
        }

        // Get host hardware info
        let cpuCount = ProcessInfo.processInfo.processorCount
        let memoryBytes = Int64(ProcessInfo.processInfo.physicalMemory)

        // Register this node with the control plane
        var nodeId: String?
        logger.info("registering node with control plane...")
        do {
            let node = try await api.registerNode(.init(
                name: effectiveNodeName,
                host_os: "macos",
                host_arch: currentArch(),
                cpu_cores: cpuCount,
                memory_bytes: memoryBytes,
                disk_bytes_total: diskSpace()
            ))
            nodeId = node.id
            logger.info("registered as node \(node.id)")
        } catch let error as APIError {
            // If registration fails with 403, the token user might not be admin
            // If 409 or similar, the node might already exist
            logger.error("failed to register node: \(error)")
            logger.info("continuing — node may already be registered")
        } catch {
            logger.error("failed to register node: \(error)")
        }

        // Main poll loop
        logger.info("entering poll loop (interval: \(effectivePollInterval)s)")
        while !Task.isCancelled {
            await pollCycle(
                api: api,
                vmManager: vmManager,
                ipswRestore: ipswRestore,
                nodeId: nodeId,
                logger: logger
            )

            try await Task.sleep(nanoseconds: UInt64(effectivePollInterval) * 1_000_000_000)
        }
    }
}

private func pollCycle(
    api: APIClient,
    vmManager: VMManager,
    ipswRestore: IPSWRestore,
    nodeId: String?,
    logger: Logger
) async {
    do {
        let environments = try await api.listEnvironments()

        for env in environments {
            // Only handle macOS environments assigned to this node
            guard env.provider == "macos" || env.provider == "virtualization" else { continue }
            if let nodeId = nodeId, env.node_id != nodeId { continue }

            switch env.state {
            case "creating":
                // Skip if VM already exists locally
                guard vmManager.vmState(envId: env.id) == nil else { continue }

                logger.info("[\(env.id)] creating VM")
                do {
                    let bundlePath = vmManager.bundlePath(envId: env.id)

                    // If not installed yet, perform IPSW restore first
                    if !ipswRestore.isInstalled(bundlePath: bundlePath) {
                        logger.info("[\(env.id)] no OS installed, performing IPSW restore...")
                        let (ipswURL, restoreImage) = try await ipswRestore.downloadLatestIPSW()
                        try await ipswRestore.installMacOS(
                            bundlePath: bundlePath,
                            ipswURL: ipswURL,
                            restoreImage: restoreImage
                        )
                        logger.info("[\(env.id)] macOS installed successfully")
                    }

                    try await vmManager.createVM(envId: env.id)
                    let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                    logger.info("[\(env.id)] VM running")
                } catch {
                    logger.error("[\(env.id)] failed to create VM: \(error)")
                    let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                }

            case "suspending":
                logger.info("[\(env.id)] suspending VM")
                do {
                    try await vmManager.suspendVM(envId: env.id)
                    let _ = try await api.updateEnvironmentState(envId: env.id, state: "suspended")
                    logger.info("[\(env.id)] VM suspended")
                } catch {
                    logger.error("[\(env.id)] failed to suspend: \(error)")
                    let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                }

            case "resuming":
                logger.info("[\(env.id)] resuming VM")
                do {
                    try await vmManager.resumeVM(envId: env.id)
                    let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                    logger.info("[\(env.id)] VM resumed")
                } catch {
                    logger.error("[\(env.id)] failed to resume: \(error)")
                    let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                }

            case "rebooting":
                logger.info("[\(env.id)] rebooting VM")
                do {
                    try await vmManager.rebootVM(envId: env.id, force: false)
                    let _ = try await api.updateEnvironmentState(envId: env.id, state: "running")
                    logger.info("[\(env.id)] VM rebooted")
                } catch {
                    logger.error("[\(env.id)] failed to reboot: \(error)")
                    let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                }

            case "destroying":
                logger.info("[\(env.id)] destroying VM")
                do {
                    try await vmManager.destroyVM(envId: env.id)
                    try await api.deleteEnvironment(envId: env.id)
                    logger.info("[\(env.id)] VM destroyed")
                } catch {
                    logger.error("[\(env.id)] failed to destroy: \(error)")
                    let _ = try? await api.updateEnvironmentState(envId: env.id, state: "failed")
                }

            default:
                break
            }
        }
    } catch {
        logger.error("poll error: \(error)")
    }
}

// MARK: - System info helpers

func currentArch() -> String {
    #if arch(arm64)
    return "arm64"
    #elseif arch(x86_64)
    return "x86_64"
    #else
    return "unknown"
    #endif
}

func diskSpace() -> Int64 {
    let homeURL = URL(fileURLWithPath: NSHomeDirectory())
    guard let values = try? homeURL.resourceValues(forKeys: [.volumeTotalCapacityKey]),
          let total = values.volumeTotalCapacity else {
        return 0
    }
    return Int64(total)
}
