import Foundation
import Logging

/// Manages TCP port forwarding from host LAN ports to VM NAT IPs via socat.
final class PortForwarder {
    private let logger = Logger(label: "orion.node-agent.port-forwarder")

    struct Forwarding {
        let vmIP: String
        let sshHostPort: Int
        let vncHostPort: Int
        var sshProcess: Process?
        var vncProcess: Process?
    }

    private var forwardings: [String: Forwarding] = [:]  // keyed by envId
    private var nextSSHPort = 10022
    private var nextVNCPort = 15900

    // MARK: - Host LAN IP

    /// Discover the host's LAN IP address (first non-loopback IPv4).
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
                    // Prefer en0/en1 (Wi-Fi/Ethernet)
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
            // Try DNS resolution of the .local hostname
            let fqdn = hostname.hasSuffix(".local") ? hostname : "\(hostname).local"
            if let ip = resolveHostname(fqdn) {
                logger.info("resolved \(fqdn) to \(ip) (attempt \(attempt))")
                return ip
            }
            // Also try scanning DHCP leases by hostname
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
        // Parse blocks between { and }, collecting name and ip_address
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
    func startForwarding(envId: String, vmIP: String) throws -> (sshPort: Int, vncPort: Int) {
        // If already forwarding, stop first
        stopForwarding(envId: envId)

        let sshPort = allocatePort(starting: &nextSSHPort)
        let vncPort = allocatePort(starting: &nextVNCPort)

        let sshProc = try spawnForwarder(listenPort: sshPort, targetHost: vmIP, targetPort: 22)
        let vncProc = try spawnForwarder(listenPort: vncPort, targetHost: vmIP, targetPort: 5900)

        forwardings[envId] = Forwarding(
            vmIP: vmIP,
            sshHostPort: sshPort,
            vncHostPort: vncPort,
            sshProcess: sshProc,
            vncProcess: vncProc
        )

        logger.info("[\(envId)] port forwarding started: SSH=\(sshPort)→\(vmIP):22, VNC=\(vncPort)→\(vmIP):5900")
        return (sshPort, vncPort)
    }

    /// Stop port forwarding for an environment.
    func stopForwarding(envId: String) {
        guard let fwd = forwardings.removeValue(forKey: envId) else { return }
        fwd.sshProcess?.terminate()
        fwd.vncProcess?.terminate()
        logger.info("[\(envId)] port forwarding stopped")
    }

    /// Stop all active forwardings (for cleanup on shutdown).
    func stopAll() {
        for (envId, fwd) in forwardings {
            fwd.sshProcess?.terminate()
            fwd.vncProcess?.terminate()
            logger.info("[\(envId)] port forwarding stopped (shutdown)")
        }
        forwardings.removeAll()
    }

    /// Check if forwarding is active for an environment.
    func isForwarding(envId: String) -> Bool {
        return forwardings[envId] != nil
    }

    // MARK: - Internals

    private func spawnForwarder(listenPort: Int, targetHost: String, targetPort: Int) throws -> Process {
        let script = """
        import socket, threading, sys, signal
        signal.signal(signal.SIGTERM, lambda *a: sys.exit(0))
        def forward(src, dst):
            try:
                while True:
                    d = src.recv(65536)
                    if not d: break
                    dst.sendall(d)
            except: pass
            finally: src.close(); dst.close()
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(('0.0.0.0', \(listenPort)))
        srv.listen(128)
        while True:
            c, _ = srv.accept()
            try:
                r = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                r.settimeout(10)
                r.connect(('\(targetHost)', \(targetPort)))
                r.settimeout(None)
                threading.Thread(target=forward, args=(c, r), daemon=True).start()
                threading.Thread(target=forward, args=(r, c), daemon=True).start()
            except: c.close()
        """
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = ["-c", script]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        return process
    }

    private func allocatePort(starting: inout Int) -> Int {
        let port = starting
        starting += 1
        return port
    }
}
