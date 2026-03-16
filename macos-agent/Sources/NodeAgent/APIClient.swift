import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import Logging

/// Client for communicating with the orion-complex control plane.
actor APIClient {
    private let baseURL: URL
    private let token: String
    private let session: URLSession
    private let logger = Logger(label: "orion.node-agent.api")

    init(baseURL: URL, token: String) {
        self.baseURL = baseURL
        self.token = token
        self.session = URLSession(configuration: .default)
    }

    // MARK: - Node registration

    struct RegisterNodeRequest: Codable {
        let name: String
        let host_os: String
        let host_arch: String
        let cpu_cores: Int
        let memory_bytes: Int64
        let disk_bytes_total: Int64
    }

    struct Node: Codable {
        let id: String
        let name: String?
        let online: Int?
    }

    func registerNode(_ req: RegisterNodeRequest) async throws -> Node {
        return try await post("/v1/nodes", body: req)
    }

    // MARK: - Environment operations

    struct Environment: Codable {
        let id: String
        let image_id: String?
        let owner_user_id: String?
        let node_id: String?
        let provider: String?
        let guest_os: String?
        let guest_arch: String?
        let state: String?
        let created_at: Int64?
        let expires_at: Int64?
        let port_forwarding: Int?
        let ssh_host: String?
        let ssh_port: Int?
        let vnc_host: String?
        let vnc_port: Int?
        let iso_url: String?
        let capture_image_id: String?
        let bypass_hw_check: Int?
        let win_install_options: String?
    }

    func listEnvironments() async throws -> [Environment] {
        return try await get("/v1/environments")
    }

    struct UpdateStateRequest: Codable {
        let state: String
    }

    func updateEnvironmentState(envId: String, state: String) async throws -> Environment {
        return try await put("/v1/environments/\(envId)/state", body: UpdateStateRequest(state: state))
    }

    func deleteEnvironment(envId: String) async throws {
        try await deleteRequest("/v1/environments/\(envId)")
    }

    // MARK: - Port forwarding endpoints

    struct UpdateEndpointsRequest: Codable {
        let ssh_host: String?
        let ssh_port: Int?
        let vnc_host: String?
        let vnc_port: Int?
    }

    func updateEndpoints(envId: String, endpoints: UpdateEndpointsRequest) async throws -> Environment {
        return try await put("/v1/environments/\(envId)/endpoints", body: endpoints)
    }

    // MARK: - Node heartbeat

    func sendHeartbeat(nodeId: String) async throws {
        try await postNoContent("/v1/nodes/\(nodeId)/heartbeat", body: EmptyBody())
    }

    // MARK: - SSH keys for provisioning

    struct UserSSHKey: Codable {
        let id: String
        let user_id: String?
        let public_key: String?
        let fingerprint: String?
    }

    func listSSHKeys() async throws -> [UserSSHKey] {
        return try await get("/v1/ssh-keys")
    }

    func listUserSSHKeys(userId: String) async throws -> [UserSSHKey] {
        return try await get("/v1/users/\(userId)/ssh-keys")
    }

    // MARK: - Snapshots

    struct Snapshot: Codable {
        let id: String
        let env_id: String?
        let name: String?
        let created_at: Int64?
    }

    func listSnapshots(envId: String) async throws -> [Snapshot] {
        return try await get("/v1/environments/\(envId)/snapshots")
    }

    // MARK: - Image info

    struct Image: Codable {
        let id: String
        let name: String?
        let provider: String?
        let guest_os: String?
        let guest_arch: String?
        let base_image_id: String?
    }

    func getImage(imageId: String) async throws -> Image {
        return try await get("/v1/images/\(imageId)")
    }

    private struct EmptyBody: Encodable {}

    // MARK: - HTTP helpers

    private func get<T: Decodable>(_ path: String) async throws -> T {
        let url = baseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (data, response) = try await session.data(for: request)
        try checkResponse(response, data: data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func post<T: Decodable, B: Encodable>(_ path: String, body: B) async throws -> T {
        let url = baseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)

        let (data, response) = try await session.data(for: request)
        try checkResponse(response, data: data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func put<T: Decodable, B: Encodable>(_ path: String, body: B) async throws -> T {
        let url = baseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = "PUT"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)

        let (data, response) = try await session.data(for: request)
        try checkResponse(response, data: data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func postNoContent<B: Encodable>(_ path: String, body: B) async throws {
        let url = baseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)

        let (data, response) = try await session.data(for: request)
        try checkResponse(response, data: data)
    }

    private func deleteRequest(_ path: String) async throws {
        let url = baseURL.appendingPathComponent(path)
        var request = URLRequest(url: url)
        request.httpMethod = "DELETE"
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")

        let (data, response) = try await session.data(for: request)
        try checkResponse(response, data: data)
    }

    private func checkResponse(_ response: URLResponse, data: Data) throws {
        guard let http = response as? HTTPURLResponse else {
            throw APIError.invalidResponse
        }
        guard (200...299).contains(http.statusCode) else {
            let body = String(data: data, encoding: .utf8) ?? ""
            logger.error("API error \(http.statusCode): \(body)")
            throw APIError.httpError(statusCode: http.statusCode, body: body)
        }
    }
}

enum APIError: Error {
    case invalidResponse
    case httpError(statusCode: Int, body: String)
}
