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
                host_arch: NodeAgentHelpers.currentArch(),
                cpu_cores: cpuCount,
                memory_bytes: memoryBytes,
                disk_bytes_total: NodeAgentHelpers.diskSpace()
            ))
            nodeId = node.id
            logger.info("registered as node \(node.id)")
        } catch let error as APIError {
            logger.error("failed to register node: \(error)")
            logger.info("continuing — node may already be registered")
        } catch {
            logger.error("failed to register node: \(error)")
        }

        // Main poll loop
        logger.info("entering poll loop (interval: \(effectivePollInterval)s)")
        var heartbeatCounter = 0
        let heartbeatEveryN = max(1, 30 / effectivePollInterval) // ~30s between heartbeats

        while !Task.isCancelled {
            // Send heartbeat periodically
            heartbeatCounter += 1
            if heartbeatCounter >= heartbeatEveryN, let nid = nodeId {
                heartbeatCounter = 0
                do {
                    try await api.sendHeartbeat(nodeId: nid)
                } catch {
                    logger.warning("heartbeat failed: \(error)")
                }
            }

            await NodeAgentHelpers.pollCycle(
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
