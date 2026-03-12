import Foundation

struct AgentConfig {
    let controlPlaneURL: URL
    let nodeName: String
    let nodeId: String?
    let apiToken: String
    let bundleStorePath: String
    let pollIntervalSeconds: Int

    static func fromEnvironment() -> AgentConfig {
        let controlPlane = ProcessInfo.processInfo.environment["ORION_CONTROL_PLANE"]
            ?? "http://127.0.0.1:3000"
        let nodeName = ProcessInfo.processInfo.environment["ORION_NODE_NAME"]
            ?? Host.current().localizedName ?? "macos-node"
        let nodeId = ProcessInfo.processInfo.environment["ORION_NODE_ID"]
        let apiToken = ProcessInfo.processInfo.environment["ORION_API_TOKEN"] ?? ""
        let bundleStore = ProcessInfo.processInfo.environment["ORION_BUNDLE_STORE"]
            ?? NSHomeDirectory() + "/.orion/bundles"
        let pollInterval = Int(ProcessInfo.processInfo.environment["ORION_POLL_INTERVAL"] ?? "5") ?? 5

        return AgentConfig(
            controlPlaneURL: URL(string: controlPlane)!,
            nodeName: nodeName,
            nodeId: nodeId,
            apiToken: apiToken,
            bundleStorePath: bundleStore,
            pollIntervalSeconds: pollInterval
        )
    }
}
