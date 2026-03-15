import Foundation
import Logging

/// Manages TCP port forwarding from host LAN ports to VM NAT IPs.
///
/// VMs run on bridge100 (192.168.64.x), which is not routable from the LAN.
/// macOS 15+ Local Network Privacy blocks non-system binaries from connecting
/// to bridge100. We work around this by:
///   - Listening on 0.0.0.0 in the Swift process (LAN-accessible)
///   - Spawning /usr/bin/nc (system binary, exempt from LNP) per accepted
///     connection to relay data to the VM
final class PortForwarder {
    private let logger = Logger(label: "orion.node-agent.port-forwarder")

    struct Forwarding {
        let vmIP: String
        let sshHostPort: Int
        let vncHostPort: Int
        var sshListener: Task<Void, Never>?
        var vncListener: Task<Void, Never>?
    }

    private var forwardings: [String: Forwarding] = [:]  // keyed by envId
    private var nextSSHPort = 10022
    private var nextVNCPort = 15900

    // MARK: - Host LAN IP

    /// Discover the host's LAN IP address (first non-loopback IPv4 on en*).
    static func hostLANIP() -> String? {
        var ifaddr: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifaddr) == 0 else { return nil }
        defer { freeifaddrs(ifaddr) }

        var current = ifaddr
        while let ifa = current {
            let flags = Int32(ifa.pointee.ifa_flags)
            let isUp = (flags & IFF_UP) != 0
            let isLoopback = (flags & IFF_LOOPBACK) != 0

            if isUp && !isLoopback,
               let addr = ifa.pointee.ifa_addr,
               addr.pointee.sa_family == UInt8(AF_INET) {
                var hostname = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                if getnameinfo(addr, socklen_t(addr.pointee.sa_len),
                               &hostname, socklen_t(hostname.count),
                               nil, 0, NI_NUMERICHOST) == 0 {
                    let ip = String(cString: hostname)
                    let name = String(cString: ifa.pointee.ifa_name)
                    if name.hasPrefix("en") {
                        return ip
                    }
                }
            }
            current = ifa.pointee.ifa_next
        }
        return nil
    }

    // MARK: - VM IP Discovery

    /// Discover a VM's NAT IP by parsing /var/db/dhcpd_leases and matching its MAC address.
    func discoverVMIP(macAddress: String, retries: Int = 30, interval: TimeInterval = 2) async -> String? {
        let normalizedMAC = macAddress.lowercased()
        for attempt in 1...retries {
            if let ip = parseLeaseFileByMAC(normalizedMAC) {
                logger.info("discovered VM IP \(ip) for MAC \(normalizedMAC) (attempt \(attempt))")
                return ip
            }
            try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
        }
        logger.warning("failed to discover VM IP for MAC \(normalizedMAC) after \(retries) attempts")
        return nil
    }

    /// Discover a VM's NAT IP by resolving its mDNS hostname.
    func discoverVMIPByHostname(hostname: String, retries: Int = 15, interval: TimeInterval = 2) async -> String? {
        for attempt in 1...retries {
            let fqdn = hostname.hasSuffix(".local") ? hostname : "\(hostname).local"
            if let ip = resolveHostname(fqdn) {
                logger.info("resolved \(fqdn) to \(ip) (attempt \(attempt))")
                return ip
            }
            if let ip = parseLeaseFileByName(hostname.replacingOccurrences(of: ".local", with: "")) {
                logger.info("found \(hostname) in DHCP leases: \(ip) (attempt \(attempt))")
                return ip
            }
            try? await Task.sleep(nanoseconds: UInt64(interval * 1_000_000_000))
        }
        logger.warning("failed to discover VM IP for hostname \(hostname) after \(retries) attempts")
        return nil
    }

    private func resolveHostname(_ hostname: String) -> String? {
        var hints = addrinfo()
        hints.ai_family = AF_INET
        hints.ai_socktype = SOCK_STREAM
        var result: UnsafeMutablePointer<addrinfo>?
        guard getaddrinfo(hostname, nil, &hints, &result) == 0, let res = result else {
            return nil
        }
        defer { freeaddrinfo(res) }
        if let addr = res.pointee.ai_addr {
            var hostname = [CChar](repeating: 0, count: Int(NI_MAXHOST))
            if getnameinfo(addr, res.pointee.ai_addrlen,
                           &hostname, socklen_t(hostname.count),
                           nil, 0, NI_NUMERICHOST) == 0 {
                return String(cString: hostname)
            }
        }
        return nil
    }

    private func parseLeaseFileByMAC(_ mac: String) -> String? {
        guard let content = try? String(contentsOfFile: "/var/db/dhcpd_leases", encoding: .utf8) else {
            return nil
        }
        var currentIP: String?
        var currentMAC: String?
        for line in content.components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed == "{" {
                currentIP = nil
                currentMAC = nil
            } else if trimmed == "}" {
                if currentMAC == mac, let ip = currentIP {
                    return ip
                }
            } else if trimmed.hasPrefix("ip_address=") {
                currentIP = String(trimmed.dropFirst("ip_address=".count))
            } else if trimmed.hasPrefix("hw_address=") {
                let hwAddr = String(trimmed.dropFirst("hw_address=".count))
                let addrPart = hwAddr.contains(",") ? String(hwAddr.split(separator: ",").last ?? "") : hwAddr
                currentMAC = addrPart.lowercased()
            }
        }
        return nil
    }

    private func parseLeaseFileByName(_ name: String) -> String? {
        guard let content = try? String(contentsOfFile: "/var/db/dhcpd_leases", encoding: .utf8) else {
            return nil
        }
        var currentIP: String?
        var currentName: String?
        for line in content.components(separatedBy: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed == "{" {
                currentIP = nil
                currentName = nil
            } else if trimmed == "}" {
                if currentName == name, let ip = currentIP {
                    return ip
                }
            } else if trimmed.hasPrefix("ip_address=") {
                currentIP = String(trimmed.dropFirst("ip_address=".count))
            } else if trimmed.hasPrefix("name=") {
                currentName = String(trimmed.dropFirst("name=".count))
            }
        }
        return nil
    }

    // MARK: - Forwarding lifecycle

    /// Start port forwarding for an environment. Returns (sshPort, vncPort).
    func startForwarding(envId: String, vmIP: String) -> (sshPort: Int, vncPort: Int) {
        // If already forwarding, stop first
        stopForwarding(envId: envId)

        let sshPort = allocatePort(starting: &nextSSHPort)
        let vncPort = allocatePort(starting: &nextVNCPort)

        let sshListener = startListener(listenPort: sshPort, targetHost: vmIP, targetPort: 22, label: "\(envId)/ssh")
        let vncListener = startListener(listenPort: vncPort, targetHost: vmIP, targetPort: 5900, label: "\(envId)/vnc")

        forwardings[envId] = Forwarding(
            vmIP: vmIP,
            sshHostPort: sshPort,
            vncHostPort: vncPort,
            sshListener: sshListener,
            vncListener: vncListener
        )

        logger.info("[\(envId)] port forwarding started: SSH=0.0.0.0:\(sshPort)→\(vmIP):22, VNC=0.0.0.0:\(vncPort)→\(vmIP):5900")
        return (sshPort, vncPort)
    }

    /// Stop port forwarding for an environment.
    func stopForwarding(envId: String) {
        guard let fwd = forwardings.removeValue(forKey: envId) else { return }
        fwd.sshListener?.cancel()
        fwd.vncListener?.cancel()
        logger.info("[\(envId)] port forwarding stopped")
    }

    /// Stop all active forwardings (for cleanup on shutdown).
    func stopAll() {
        for (envId, fwd) in forwardings {
            fwd.sshListener?.cancel()
            fwd.vncListener?.cancel()
            logger.info("[\(envId)] port forwarding stopped (shutdown)")
        }
        forwardings.removeAll()
    }

    /// Check if forwarding is active for an environment.
    func isForwarding(envId: String) -> Bool {
        return forwardings[envId] != nil
    }

    // MARK: - TCP Listener + nc bridge

    /// Start a TCP listener on 0.0.0.0:listenPort.
    /// For each accepted connection, spawn /usr/bin/nc to relay to targetHost:targetPort.
    private func startListener(listenPort: Int, targetHost: String, targetPort: Int, label: String) -> Task<Void, Never> {
        let logger = self.logger
        return Task.detached {
            let serverFD = socket(AF_INET, SOCK_STREAM, 0)
            guard serverFD >= 0 else {
                logger.error("[\(label)] failed to create server socket")
                return
            }

            var reuseAddr: Int32 = 1
            setsockopt(serverFD, SOL_SOCKET, SO_REUSEADDR, &reuseAddr, socklen_t(MemoryLayout<Int32>.size))

            var addr = sockaddr_in()
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = UInt16(listenPort).bigEndian
            addr.sin_addr.s_addr = INADDR_ANY

            let bindResult = withUnsafePointer(to: &addr) { ptr in
                ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                    bind(serverFD, sockPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard bindResult == 0 else {
                logger.error("[\(label)] bind failed on port \(listenPort): errno \(errno)")
                close(serverFD)
                return
            }

            guard listen(serverFD, 16) == 0 else {
                logger.error("[\(label)] listen failed: errno \(errno)")
                close(serverFD)
                return
            }

            logger.info("[\(label)] listening on 0.0.0.0:\(listenPort)")

            while !Task.isCancelled {
                var clientAddr = sockaddr_in()
                var clientAddrLen = socklen_t(MemoryLayout<sockaddr_in>.size)
                let clientFD = withUnsafeMutablePointer(to: &clientAddr) { ptr in
                    ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                        accept(serverFD, sockPtr, &clientAddrLen)
                    }
                }

                if clientFD < 0 {
                    if Task.isCancelled { break }
                    continue
                }

                // Handle each connection in a detached task
                let tHost = targetHost
                let tPort = targetPort
                let connLabel = label
                Task.detached {
                    await Self.handleConnection(clientFD: clientFD, targetHost: tHost, targetPort: tPort, label: connLabel, logger: logger)
                }
            }

            close(serverFD)
            logger.info("[\(label)] listener stopped")
        }
    }

    /// Handle a single client connection by spawning /usr/bin/nc to the target.
    private static func handleConnection(clientFD: Int32, targetHost: String, targetPort: Int, label: String, logger: Logger) async {
        // Spawn /usr/bin/nc to connect to the VM (system binary, bypasses Local Network Privacy)
        let ncProcess = Process()
        ncProcess.executableURL = URL(fileURLWithPath: "/usr/bin/nc")
        ncProcess.arguments = [targetHost, String(targetPort)]

        let ncStdinPipe = Pipe()
        let ncStdoutPipe = Pipe()
        ncProcess.standardInput = ncStdinPipe
        ncProcess.standardOutput = ncStdoutPipe
        ncProcess.standardError = FileHandle.nullDevice

        do {
            try ncProcess.run()
        } catch {
            logger.error("[\(label)] failed to spawn nc: \(error)")
            close(clientFD)
            return
        }

        let ncStdinHandle = ncStdinPipe.fileHandleForWriting
        let ncStdoutHandle = ncStdoutPipe.fileHandleForReading

        // Bidirectional relay: client ↔ nc
        // Direction 1: client socket → nc stdin
        let clientToNC = Task.detached {
            let bufSize = 65536
            let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: bufSize)
            defer { buf.deallocate() }

            while true {
                let bytesRead = read(clientFD, buf, bufSize)
                if bytesRead <= 0 { break }
                let data = Data(bytes: buf, count: bytesRead)
                do {
                    try ncStdinHandle.write(contentsOf: data)
                } catch {
                    break
                }
            }
            try? ncStdinHandle.close()
        }

        // Direction 2: nc stdout → client socket
        let ncToClient = Task.detached {
            let bufSize = 65536
            let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: bufSize)
            defer { buf.deallocate() }

            let fd = ncStdoutHandle.fileDescriptor
            while true {
                let bytesRead = read(fd, buf, bufSize)
                if bytesRead <= 0 { break }
                var totalWritten = 0
                while totalWritten < bytesRead {
                    let written = write(clientFD, buf.advanced(by: totalWritten), bytesRead - totalWritten)
                    if written <= 0 { return }
                    totalWritten += written
                }
            }
        }

        // Wait for either direction to finish
        _ = await clientToNC.value
        _ = await ncToClient.value

        // Cleanup
        close(clientFD)
        if ncProcess.isRunning {
            ncProcess.terminate()
        }
    }

    // MARK: - Helpers

    private func allocatePort(starting: inout Int) -> Int {
        while true {
            let port = starting
            starting += 1
            if starting > 65000 { starting = 10000 }
            if isPortAvailable(port) { return port }
        }
    }

    private func isPortAvailable(_ port: Int) -> Bool {
        let sock = socket(AF_INET, SOCK_STREAM, 0)
        guard sock >= 0 else { return false }
        defer { close(sock) }
        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = UInt16(port).bigEndian
        addr.sin_addr.s_addr = INADDR_ANY
        var reuseAddr: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &reuseAddr, socklen_t(MemoryLayout<Int32>.size))
        return withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0
            }
        }
    }
}
