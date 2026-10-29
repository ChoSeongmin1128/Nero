import Combine
import CryptoKit
import Foundation
import Security

struct NotionOAuthGrant: Codable, Equatable {
    let id: UUID
    let clientID: String
    let metadata: NotionOAuthMetadata
    var accessToken: String
    var refreshToken: String
    var expiresAt: Date
    let workspaceID: String
    let userID: String
}

enum NotionStoredConnection: Codable, Equatable {
    case disconnected
    case manual(String)
    case oauth(NotionOAuthGrant)
}

@MainActor
protocol NotionConnectionStoring {
    func read() throws -> NotionStoredConnection?
    func save(_ connection: NotionStoredConnection) throws
    func clientID() throws -> String?
    func saveClientID(_ id: String) throws
}

@MainActor
final class MemoryNotionConnectionStore: NotionConnectionStoring {
    var value: NotionStoredConnection?
    var registration: String?
    func read() throws -> NotionStoredConnection? { value }
    func save(_ connection: NotionStoredConnection) throws { value = connection }
    func clientID() throws -> String? { registration }
    func saveClientID(_ id: String) throws { registration = id }
}

struct NotionKeychainItem {
    let account: String
    private var query: [String: Any] { [kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "io.github.ChoSeongmin1128.Grove.Notion", kSecAttrAccount as String: account] }
    func read() throws -> Data? {
        var values = query
        values[kSecReturnData as String] = true; values[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(values as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw NotionExportError.keychain }
        return data
    }
    func save(_ data: Data) throws {
        let status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound {
            var values = query; values[kSecValueData as String] = data
            values[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            guard SecItemAdd(values as CFDictionary, nil) == errSecSuccess else { throw NotionExportError.keychain }
        } else if status != errSecSuccess { throw NotionExportError.keychain }
    }
    func delete() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw NotionExportError.keychain }
    }
}

@MainActor
struct KeychainNotionConnectionStore: NotionConnectionStoring {
    private let connection = NotionKeychainItem(account: "connection")
    private let registration = NotionKeychainItem(account: "mcp-client-v1")
    func read() throws -> NotionStoredConnection? {
        guard let data = try connection.read() else { return nil }
        return try Self.decode(data)
    }
    func save(_ value: NotionStoredConnection) throws {
        if value == .disconnected { try connection.delete() }
        else { try connection.save(JSONEncoder().encode(value)) }
    }
    func clientID() throws -> String? { try registration.read().flatMap { String(data: $0, encoding: .utf8) } }
    func saveClientID(_ id: String) throws { try registration.save(Data(id.utf8)) }
    static func decode(_ data: Data) throws -> NotionStoredConnection {
        if data.first == UInt8(ascii: "{") { return try JSONDecoder().decode(NotionStoredConnection.self, from: data) }
        guard let token = String(data: data, encoding: .utf8), !token.isEmpty, !token.contains(where: \.isWhitespace) else { throw NotionExportError.keychain }
        return .manual(token)
    }
}

@MainActor
final class NotionConnection: ObservableObject {
    @Published private(set) var isConnecting = false
    @Published private(set) var isRefreshing = false
    @Published private(set) var requiresReconnect = false
    @Published private(set) var message: String?
    @Published private var stored: NotionStoredConnection = .disconnected
    private let storage: any NotionConnectionStoring
    private let service: NotionOAuthService
    private let browser: any NotionBrowserAuthorizing
    private let now: () -> Date
    private let defaults: UserDefaults
    private var connectID: UUID?
    private var refreshTask: Task<String, any Error>?
    let diagnostics: NotionDiagnostics
    private var cachedClient: (grantID: UUID, session: ObjectIdentifier?, client: NotionMCPPageClient)?

    init(storage: any NotionConnectionStoring = KeychainNotionConnectionStore(),
         service: NotionOAuthService = NotionOAuthService(), browser: (any NotionBrowserAuthorizing)? = nil,
         defaults: UserDefaults = .standard,
         diagnostics: NotionDiagnostics = NotionDiagnostics(),
         now: @escaping () -> Date = Date.init) {
        self.storage = storage; self.service = service; self.browser = browser ?? NotionBrowserAuthorization(); self.now = now; self.defaults = defaults
        self.diagnostics = diagnostics
        do { stored = try storage.read() ?? .disconnected }
        catch { message = "Notion 연결 정보를 읽지 못했습니다." }
    }

    var isConnected: Bool {
        if requiresReconnect { return false }
        return switch stored { case .disconnected: false; default: true }
    }
    var isOAuth: Bool { if case .oauth = stored { true } else { false } }
    var destinationKey: String {
        guard case .oauth(let grant) = stored else { return "notionParentLink" }
        let scope = grant.workspaceID + ":" + grant.userID
        let hash = SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined()
        return "notionParentLink." + hash
    }
    var parentLink: String {
        get { defaults.string(forKey: destinationKey) ?? "" }
        set { defaults.set(newValue, forKey: destinationKey) }
    }

    func connect() async {
        guard !isConnecting, !isRefreshing else { return }
        let id = UUID(); connectID = id; isConnecting = true; message = nil
        defer { if connectID == id { connectID = nil; isConnecting = false } }
        do {
            let metadata = try await service.discover()
            let clientID: String
            if let saved = try storage.clientID(), !saved.isEmpty { clientID = saved }
            else {
                clientID = try await service.register(metadata: metadata)
                try Task.checkCancellation()
                guard connectID == id else { return }
                try storage.saveClientID(clientID)
            }
            try Task.checkCancellation(); guard connectID == id else { return }
            let attempt = try NotionOAuthAttempt(startedAt: now())
            let callback = try await browser.authorize(url: attempt.authorizationURL(metadata: metadata, clientID: clientID))
            try Task.checkCancellation(); guard connectID == id else { return }
            let code = try attempt.authorizationCode(from: callback, now: now())
            let tokens = try await service.exchange(code: code, attempt: attempt, clientID: clientID, metadata: metadata)
            try Task.checkCancellation(); guard connectID == id else { return }
            guard let workspace = tokens.workspaceId, !workspace.isEmpty, let user = tokens.userId, !user.isEmpty,
                  let refresh = tokens.refreshToken else { throw NotionConnectionError.invalidResponse }
            let grant = NotionOAuthGrant(id: UUID(), clientID: clientID, metadata: metadata, accessToken: tokens.accessToken,
                refreshToken: refresh, expiresAt: now().addingTimeInterval(tokens.expiresIn), workspaceID: workspace, userID: user)
            try storage.save(.oauth(grant))
            cachedClient = nil
            stored = .oauth(grant); requiresReconnect = false
        } catch {
            if connectID == id { message = error is CancellationError ? NotionConnectionError.cancelled.localizedDescription : error.localizedDescription }
        }
    }

    func cancelConnect() { connectID = nil; isConnecting = false; browser.cancel() }

    func saveManualToken(_ token: String) throws {
        guard !isConnecting, !isRefreshing else { throw NotionExportError.busy }
        let cleaned = token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !cleaned.isEmpty, !cleaned.contains(where: \.isWhitespace) else { throw NotionExportError.missingToken }
        try storage.save(.manual(cleaned)); cachedClient = nil; stored = .manual(cleaned); requiresReconnect = false; message = nil
    }

    func disconnect() throws {
        guard !isConnecting, !isRefreshing else { throw NotionExportError.busy }
        try storage.save(.disconnected); cachedClient = nil; stored = .disconnected; requiresReconnect = false; message = nil
    }

    func client(session: URLSession? = nil) throws -> any NotionPageClient {
        guard !requiresReconnect else { throw NotionConnectionError.reconnectRequired }
        switch stored {
        case .manual(let token): return NotionClient(token: token, session: session ?? .shared)
        case .oauth(let grant):
            let key = session.map(ObjectIdentifier.init)
            if let cachedClient, cachedClient.grantID == grant.id, cachedClient.session == key { return cachedClient.client }
            let client = NotionMCPPageClient(connection: self, grantID: grant.id, session: session ?? NotionNoRedirects.session())
            cachedClient = (grant.id, key, client)
            return client
        case .disconnected: throw NotionExportError.missingToken
        }
    }

    func accessToken(grantID: UUID, rejectedToken: String? = nil) async throws -> String {
        guard !requiresReconnect, case .oauth(let grant) = stored, grant.id == grantID else { throw NotionConnectionError.connectionChanged }
        if let refreshTask { return try await refreshTask.value }
        if let rejectedToken, rejectedToken != grant.accessToken { return grant.accessToken }
        if rejectedToken == nil && grant.expiresAt.timeIntervalSince(now()) > 60 { return grant.accessToken }
        isRefreshing = true
        let task = Task { @MainActor in
            let startedAt = Date(), started = ProcessInfo.processInfo.systemUptime
            var succeeded = false
            defer {
                diagnostics.record(kind: .authentication, operation: "oauth-refresh", startedAt: startedAt,
                    elapsed: ProcessInfo.processInfo.systemUptime - started, failed: !succeeded)
            }
            do {
                let tokens = try await service.refresh(grant)
                guard case .oauth(let current) = stored, current.id == grantID else { throw NotionConnectionError.connectionChanged }
                var updated = current
                updated.accessToken = tokens.accessToken; updated.refreshToken = tokens.refreshToken!
                updated.expiresAt = now().addingTimeInterval(tokens.expiresIn)
                // Persist the rotated pair together before publishing or using either token.
                do { try storage.save(.oauth(updated)) }
                catch {
                    // The server already rotated the pair, so the stored refresh token is dead.
                    cachedClient = nil
                    requiresReconnect = true
                    message = NotionConnectionError.reconnectRequired.localizedDescription
                    throw error
                }
                stored = .oauth(updated)
                succeeded = true
                return updated.accessToken
            } catch NotionConnectionError.reconnectRequired {
                // Only a rejected refresh token ends the grant. Network or server failures
                // fail this request and leave the stored connection usable for a retry.
                cachedClient = nil
                requiresReconnect = true
                message = NotionConnectionError.reconnectRequired.localizedDescription
                try storage.save(.disconnected); stored = .disconnected
                throw NotionConnectionError.reconnectRequired
            } catch NotionConnectionError.connectionChanged {
                cachedClient = nil
                throw NotionConnectionError.connectionChanged
            }
        }
        refreshTask = task
        defer { refreshTask = nil; isRefreshing = false }
        return try await task.value
    }

    func validateGrant(id: UUID) throws {
        guard !requiresReconnect, case .oauth(let grant) = stored, grant.id == id else {
            throw NotionConnectionError.connectionChanged
        }
    }
}
