import AuthenticationServices
import CryptoKit
import Foundation
import Testing
@testable import GroveApp

private final class OAuthFixtureProtocol: URLProtocol, @unchecked Sendable {
    static let lock = NSLock()
    nonisolated(unsafe) static var invalidRefresh = false
    nonisolated(unsafe) static var unavailableRefresh = false
    nonisolated(unsafe) static var workspace = "workspace-one"
    nonisolated(unsafe) static var requests: [URLRequest] = []
    nonisolated(unsafe) static var tools: [(Int, [String: Any])] = []
    nonisolated(unsafe) static var rawToolResults = false
    nonisolated(unsafe) static var failNextInitialized = false
    static var metadata: [String: Any] { ["issuer": "https://mcp.notion.com", "authorization_endpoint": "https://mcp.notion.com/authorize",
        "token_endpoint": "https://mcp.notion.com/token", "registration_endpoint": "https://mcp.notion.com/register",
        "code_challenge_methods_supported": ["S256"], "token_endpoint_auth_methods_supported": ["none"]] }
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        var recorded = request
        if recorded.httpBody == nil, let stream = recorded.httpBodyStream {
            stream.open(); defer { stream.close() }
            var data = Data(), buffer = [UInt8](repeating: 0, count: 4096)
            while stream.hasBytesAvailable {
                let count = stream.read(&buffer, maxLength: buffer.count)
                if count <= 0 { break }
                data.append(contentsOf: buffer.prefix(count))
            }
            recorded.httpBody = data
        }
        let result: (Int, [String: Any]) = Self.lock.withLock {
            Self.requests.append(recorded)
            switch recorded.url!.path {
            case "/.well-known/oauth-protected-resource/mcp": return (200, ["resource": NotionOAuthConfiguration.server.absoluteString, "authorization_servers": ["https://mcp.notion.com"]])
            case "/.well-known/oauth-authorization-server": return (200, Self.metadata)
            case "/register": return (201, ["client_id": "registered-client", "token_endpoint_auth_method": "none"])
            case "/token":
                let refresh = String(data: recorded.httpBody ?? Data(), encoding: .utf8)?.contains("grant_type=refresh_token") == true
                if refresh && Self.invalidRefresh { return (400, ["error": "invalid_grant"]) }
                if refresh && Self.unavailableRefresh { return (503, ["error": "temporarily_unavailable"]) }
                var result: [String: Any] = ["access_token": refresh ? "rotated-access" : "access", "refresh_token": refresh ? "rotated-refresh" : "refresh", "token_type": "bearer", "expires_in": 3600]
                if !refresh { result["workspace_id"] = Self.workspace; result["user_id"] = "authorized-user" }
                return (200, result)
            case "/mcp":
                let body = (try? JSONSerialization.jsonObject(with: recorded.httpBody ?? Data()) as? [String: Any]) ?? [:]
                guard let method = body["method"] as? String else { return (400, [:]) }
                if method == "initialize" { return (200, ["jsonrpc": "2.0", "id": body["id"]!, "result": ["protocolVersion": "2025-11-25", "capabilities": ["tools": [:]]]]) }
                if method == "notifications/initialized" {
                    if Self.failNextInitialized { Self.failNextInitialized = false; return (503, [:]) }
                    return (202, [:])
                }
                if method == "tools/call", let params = body["params"] as? [String: Any],
                   params["name"] as? String == "notion-update-page",
                   let arguments = params["arguments"] as? [String: Any], arguments["command"] as? String == "insert_content",
                   (arguments["content"] as? String == nil || arguments["new_str"] != nil) {
                    return (200, ["jsonrpc": "2.0", "id": body["id"]!, "error": ["code": -32602, "message": "insert_content requires content"]])
                }
                let next = Self.tools.isEmpty ? (500, [:]) : Self.tools.removeFirst()
                if next.0 != 200 { return next }
                let toolResult: [String: Any]
                if Self.rawToolResults { toolResult = next.1 }
                else { toolResult = ["structuredContent": next.1] }
                return (next.0, ["jsonrpc": "2.0", "id": body["id"]!, "result": toolResult])
            default: return (404, [:])
            }
        }
        let http = HTTPURLResponse(url: request.url!, statusCode: result.0, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json", "Mcp-Session-Id": "fixture-session"])!
        client?.urlProtocol(self, didReceive: http, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: try! JSONSerialization.data(withJSONObject: result.1))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
    static func reset() { lock.withLock { requests = []; invalidRefresh = false; unavailableRefresh = false; workspace = "workspace-one"; tools = []; rawToolResults = false; failNextInitialized = false } }
}

@MainActor
private final class OAuthFixtureBrowser: NotionBrowserAuthorizing {
    var cancels = false
    var invalidState = false
    func authorize(url: URL) async throws -> URL {
        if cancels { throw NotionConnectionError.cancelled }
        let state = URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.first { $0.name == "state" }!.value!
        var callback = URLComponents(url: NotionOAuthConfiguration.redirect, resolvingAgainstBaseURL: false)!
        callback.queryItems = [.init(name: "state", value: invalidState ? "wrong-state" : state), .init(name: "code", value: "code"), .init(name: "iss", value: "https://mcp.notion.com")]
        return callback.url!
    }
    func cancel() {}
}

@MainActor
private final class BackgroundOAuthSession: NotionAuthenticationSession {
    let completion: @Sendable (URL?, (any Error)?) -> Void
    let response: URL?
    let error: (any Error)?
    var cancelled = false

    init(completion: @escaping @Sendable (URL?, (any Error)?) -> Void, response: URL?, error: (any Error)? = nil) {
        self.completion = completion; self.response = response; self.error = error
    }

    func start(presenting context: any ASWebAuthenticationPresentationContextProviding) -> Bool {
        let completion = completion, response = response, error = error
        DispatchQueue.global(qos: .utility).async { completion(response, error) }
        return true
    }

    func cancel() { cancelled = true }
}

@MainActor
private final class FailingOAuthStore: NotionConnectionStoring {
    var value: NotionStoredConnection?
    func read() throws -> NotionStoredConnection? { value }
    func save(_ connection: NotionStoredConnection) throws { throw NotionExportError.keychain }
    func clientID() throws -> String? { "registered-client" }
    func saveClientID(_ id: String) throws {}
}

@Suite(.serialized)
@MainActor
struct NotionOAuthTests {
    private func session() -> URLSession {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [OAuthFixtureProtocol.self]
        return URLSession(configuration: config)
    }
    private func grant() throws -> NotionOAuthGrant {
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return NotionOAuthGrant(id: UUID(), clientID: "registered-client", metadata: try decoder.decode(NotionOAuthMetadata.self, from: JSONSerialization.data(withJSONObject: OAuthFixtureProtocol.metadata)),
            accessToken: "expired-access", refreshToken: "old-refresh", expiresAt: Date(timeIntervalSince1970: 0), workspaceID: "workspace-one", userID: "authorized-user")
    }
    private func defaults() -> UserDefaults { UserDefaults(suiteName: "Grove.OAuth.Tests.\(UUID().uuidString)")! }

    @Test func browserCompletionCanArriveOnABackgroundQueueAndBeRetried() async throws {
        let callback = URL(string: NotionOAuthConfiguration.redirect.absoluteString + "?state=state&code=code")!
        let browser = NotionBrowserAuthorization { _, completion in BackgroundOAuthSession(completion: completion, response: callback) }
        for _ in 0..<2 { #expect(try await browser.authorize(url: URL(string: "https://mcp.notion.com/authorize")!) == callback) }
    }

    @Test func backgroundBrowserCancellationAndOtherErrorsRemainDistinct() async throws {
        let cancelled = NSError(domain: ASWebAuthenticationSessionErrorDomain, code: ASWebAuthenticationSessionError.canceledLogin.rawValue)
        let cancellation = NotionBrowserAuthorization { _, completion in BackgroundOAuthSession(completion: completion, response: nil, error: cancelled) }
        do {
            _ = try await cancellation.authorize(url: URL(string: "https://mcp.notion.com/authorize")!)
            Issue.record("Cancellation did not fail")
        } catch {
            guard case NotionConnectionError.cancelled = error else { Issue.record("Unexpected cancellation error"); return }
        }
        let failure = NSError(domain: "Browser.Fixture", code: 42)
        let browser = NotionBrowserAuthorization { _, completion in BackgroundOAuthSession(completion: completion, response: nil, error: failure) }
        do {
            _ = try await browser.authorize(url: URL(string: "https://mcp.notion.com/authorize")!)
            Issue.record("Browser error did not fail")
        } catch { #expect((error as NSError).domain == failure.domain && (error as NSError).code == failure.code) }
    }

    @Test func pkceMatchesTheRFCVectorAndDoesNotIncludeTheVerifierInTheBrowserURL() throws {
        let attempt = try NotionOAuthAttempt(state: "state", verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
        #expect(attempt.challenge == "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        let url = try attempt.authorizationURL(metadata: grant().metadata, clientID: "client")
        #expect(!url.absoluteString.contains(attempt.verifier))
        #expect(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.contains(.init(name: "resource", value: NotionOAuthConfiguration.server.absoluteString)) == true)
    }

    @Test func wrongStateIssuerDuplicateParametersAndExpiredCallbacksAreRejected() throws {
        let attempt = try NotionOAuthAttempt(state: "correct", verifier: "verifier", startedAt: Date(timeIntervalSince1970: 0))
        for query in ["state=wrong&code=code", "state=correct&code=code&iss=https://other.example", "state=correct&state=correct&code=code"] {
            #expect(throws: (any Error).self) { try attempt.authorizationCode(from: URL(string: NotionOAuthConfiguration.redirect.absoluteString + "?" + query)!, now: Date(timeIntervalSince1970: 1)) }
        }
        #expect(throws: (any Error).self) { try attempt.authorizationCode(from: URL(string: NotionOAuthConfiguration.redirect.absoluteString + "?state=correct&code=code")!, now: Date(timeIntervalSince1970: 601)) }
        #expect(throws: (any Error).self) { try NotionOAuthConfiguration.trustedEndpoint("https://mcp.notion.com.attacker.example/token") }
    }

    @Test func legacyCredentialsRemainReadableAndMalformedStructuredCredentialsCannotBecomeBearerTokens() throws {
        #expect(try KeychainNotionConnectionStore.decode(Data("legacy-private-token".utf8)) == .manual("legacy-private-token"))
        #expect(throws: (any Error).self) { try KeychainNotionConnectionStore.decode(Data("{invalid".utf8)) }
    }

    @Test func connectStoresIdentityAndReusesRegistrationWithoutRestrictingTheNextWorkspace() async throws {
        OAuthFixtureProtocol.reset()
        let storage = MemoryNotionConnectionStore(), browser = OAuthFixtureBrowser()
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: browser, defaults: defaults())
        await connection.connect(); #expect(connection.isConnected && connection.isOAuth)
        connection.parentLink = "first-user-selected-page"
        let firstKey = connection.destinationKey
        OAuthFixtureProtocol.workspace = "workspace-two"
        await connection.connect()
        #expect(connection.isConnected && connection.destinationKey != firstKey && connection.parentLink.isEmpty)
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/register" }.count == 1)
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/token" }.allSatisfy { !(String(data: $0.httpBody ?? Data(), encoding: .utf8) ?? "").contains("client_secret") })
    }

    @Test func cancelledOrMismatchedAuthorizationPreservesThePreviousConnection() async throws {
        OAuthFixtureProtocol.reset()
        let storage = MemoryNotionConnectionStore(); storage.value = .manual("previous-token")
        let browser = OAuthFixtureBrowser(); browser.cancels = true
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: browser, defaults: defaults())
        await connection.connect(); #expect(storage.value == .manual("previous-token"))
        browser.cancels = false; browser.invalidState = true
        await connection.connect(); #expect(storage.value == .manual("previous-token"))
        #expect(!OAuthFixtureProtocol.requests.contains { $0.url?.path == "/token" })
    }

    @Test func concurrentExpiryChecksRefreshOnceAndPersistTheRotatedPairBeforeReturning() async throws {
        OAuthFixtureProtocol.reset()
        let storage = MemoryNotionConnectionStore(), grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        async let first = connection.accessToken(grantID: grant.id)
        async let second = connection.accessToken(grantID: grant.id)
        #expect(try await first == "rotated-access"); #expect(try await second == "rotated-access")
        guard case .oauth(let saved) = storage.value else { Issue.record("Missing saved grant"); return }
        #expect(saved.refreshToken == "rotated-refresh" && saved.workspaceID == grant.workspaceID)
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/token" }.count == 1)
    }

    @Test func invalidGrantStopsRefreshAndClearsDeadTokens() async throws {
        OAuthFixtureProtocol.reset(); OAuthFixtureProtocol.invalidRefresh = true
        let storage = MemoryNotionConnectionStore(), grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        await #expect(throws: (any Error).self) { try await connection.accessToken(grantID: grant.id) }
        await #expect(throws: (any Error).self) { try await connection.accessToken(grantID: grant.id) }
        #expect(storage.value == .disconnected && connection.requiresReconnect)
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/token" }.count == 1)
    }

    @Test func temporaryRefreshFailureKeepsTheGrantForARetry() async throws {
        OAuthFixtureProtocol.reset(); OAuthFixtureProtocol.unavailableRefresh = true
        let storage = MemoryNotionConnectionStore(), grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        await #expect(throws: (any Error).self) { try await connection.accessToken(grantID: grant.id) }
        #expect(storage.value == .oauth(grant) && !connection.requiresReconnect && connection.isConnected)
        OAuthFixtureProtocol.unavailableRefresh = false
        _ = try await connection.accessToken(grantID: grant.id)
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/token" }.count == 2)
    }

    @Test func rotatedTokensAreNotPublishedWhenTheirAtomicSaveFails() async throws {
        OAuthFixtureProtocol.reset()
        let storage = FailingOAuthStore(), grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        await #expect(throws: (any Error).self) { try await connection.accessToken(grantID: grant.id) }
        #expect(storage.value == .oauth(grant) && connection.requiresReconnect && !connection.isConnected)
    }

    @Test func streamableHTTPMatchesReplyIDsAndIgnoresNotifications() throws {
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":\"request\",\"result\":{\"value\":1}}\n\n"
        #expect(try NotionMCPTransport.result(data: Data(sse.utf8), contentType: "text/event-stream", id: "request")["value"] as? Int == 1)
        #expect(throws: (any Error).self) { try NotionMCPTransport.result(data: Data(sse.utf8), contentType: "text/event-stream", id: "different") }
    }

    @Test func invalidInsertArgumentsAreRejectedByTheProtocolFixture() async throws {
        OAuthFixtureProtocol.reset()
        let storage = MemoryNotionConnectionStore(); let grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        let transport = NotionMCPTransport(connection: connection, grantID: grant.id, session: session())
        do {
            _ = try await transport.call("notion-update-page", arguments: ["page_id": "01234567-89ab-cdef-0123-456789abcdef",
                "command": "insert_content", "new_str": "\n---\n", "position": ["type": "end"]], writes: true)
            Issue.record("Invalid insert_content request was accepted")
        } catch { if case NotionExportError.requestRejected = error {} else { Issue.record("Unexpected error: \(error)") } }
    }

    @Test func rpcValidationRejectionAndAnUnknownServerFailureAreDistinct() throws {
        for code in [-32601, -32602, -32000] {
            let data = try JSONSerialization.data(withJSONObject: ["jsonrpc": "2.0", "id": "request", "error": ["code": code, "message": "failure"]])
            do {
                _ = try NotionMCPTransport.result(data: data, contentType: "application/json", id: "request")
                Issue.record("RPC error was accepted")
            } catch {
                if code == -32000 { if case NotionExportError.unknownResult = error {} else { Issue.record("Unexpected error: \(error)") } }
                else { if case NotionExportError.requestRejected = error {} else { Issue.record("Unexpected error: \(error)") } }
            }
        }
    }

    @Test(arguments: ["validation_error", "internal_error"], [false, true])
    func toolValidationErrorsDoNotHideAnUncertainWrite(_ code: String, structured: Bool) async throws {
        OAuthFixtureProtocol.reset(); OAuthFixtureProtocol.rawToolResults = true
        let error = try JSONSerialization.data(withJSONObject: ["object": "error", "code": code, "message": "error"])
        let result: [String: Any] = structured
            ? ["isError": true, "structuredContent": ["error": ["code": code]]]
            : ["isError": true, "content": [["type": "text", "text": String(decoding: error, as: UTF8.self)]]]
        OAuthFixtureProtocol.tools = [(200, result)]
        let storage = MemoryNotionConnectionStore(); let grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        let transport = NotionMCPTransport(connection: connection, grantID: grant.id, session: session())
        do {
            _ = try await transport.call("notion-create-pages", arguments: [:], writes: true)
            Issue.record("Tool error was accepted")
        } catch {
            if code == "validation_error" { if case NotionExportError.requestRejected = error {} else { Issue.record("Unexpected error: \(error)") } }
            else { if case NotionExportError.unknownResult = error {} else { Issue.record("Unexpected error: \(error)") } }
        }
    }

    private func page(_ id: String, parent: String? = nil, content: String = "기존 원문") -> [String: Any] {
        let ancestors = parent.map { "<ancestor-path><parent-page url=\"\(NotionPageLink.url(for: $0))\" title=\"부모\"/></ancestor-path>" } ?? "<ancestor-path></ancestor-path>"
        return ["text": "<page url=\"\(NotionPageLink.url(for: id))\">\(ancestors)<properties>{\"title\":\"검증 페이지\"}</properties><content>\n\(content)\n</content></page>"]
    }

    private func rpcCalls() -> [[String: Any]] {
        OAuthFixtureProtocol.requests.filter { $0.url?.path == "/mcp" }
            .compactMap { try? JSONSerialization.jsonObject(with: $0.httpBody ?? Data()) as? [String: Any] }
    }

    @Test func lookupAndApplyShareInitializationButRefreshTheParentAtSaveTime() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef", child = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        OAuthFixtureProtocol.tools = [(200, page(parent, content: "조회 당시")), (200, page(parent, content: "저장 직전")),
            (200, ["page_id": parent]), (200, ["pages": [["id": child]]]), (200, page(child, parent: parent))]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let diagnostics = NotionDiagnostics()
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults(), diagnostics: diagnostics)
        let shared = session()
        _ = try await connection.client(session: shared).parentTitle(id: parent)
        #expect(rpcCalls().count == 3)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let exporter = NotionExporter(directory: directory, diagnostics: diagnostics)
        let meeting = MeetingRecord(title: "검증 회의", startedAt: Date(), duration: 1, status: .ready, glossaryProfile: "", transcript: [], claims: [])
        let document = try TranscriptDocument(speakers: [], utterances: [.init(id: UUID(), startTime: 0, endTime: 1,
            rawText: "전체 원문", sourceChannelID: "recording", engineClusterID: nil, speakerID: nil, editedText: nil)])
        _ = try await exporter.save(meeting: meeting, document: document, parentLink: parent, original: false, client: connection.client(session: shared))
        #expect(rpcCalls().count == 7)
        #expect(rpcCalls().filter { $0["method"] as? String == "initialize" }.count == 1)
        let tools = rpcCalls().compactMap { $0["params"] as? [String: Any] }.compactMap { $0["name"] as? String }
        #expect(tools == ["notion-fetch", "notion-fetch", "notion-update-page", "notion-create-pages", "notion-fetch"])
        #expect(diagnostics.entries.filter { $0.kind == .request }.count == 7)
        #expect(diagnostics.entries.last?.kind == .export && diagnostics.entries.last?.failed == false)
        #expect(exporter.stage == nil && !exporter.isSaving)
    }

    @Test func defaultClientIsReusedAndDisconnectInvalidatesItWithoutRetainingConnection() async throws {
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        var connection: NotionConnection? = NotionConnection(storage: storage, defaults: defaults())
        weak var reference = connection
        let first = try #require(try connection?.client() as? NotionMCPPageClient)
        let second = try #require(try connection?.client() as? NotionMCPPageClient)
        #expect(first === second)
        try connection?.disconnect()
        await #expect(throws: NotionConnectionError.self) { _ = try await first.parentTitle(id: "01234567-89ab-cdef-0123-456789abcdef") }
        connection = nil
        #expect(reference == nil)
    }

    @Test func expiredReadSessionInitializesOnceAndRetriesOnlyTheRead() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef"
        OAuthFixtureProtocol.tools = [(200, page(parent)), (404, [:]), (200, page(parent))]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults())
        let client = try connection.client(session: session())
        _ = try await client.parentTitle(id: parent)
        _ = try await client.parentTitle(id: parent)
        #expect(rpcCalls().filter { $0["method"] as? String == "initialize" }.count == 2)
        let initializeRequests = OAuthFixtureProtocol.requests.filter { String(data: $0.httpBody ?? Data(), encoding: .utf8)?.contains("\"method\":\"initialize\"") == true }
        #expect(initializeRequests.allSatisfy { $0.value(forHTTPHeaderField: "Mcp-Session-Id") == nil })
        #expect(rpcCalls().compactMap { $0["params"] as? [String: Any] }.compactMap { $0["name"] as? String } == ["notion-fetch", "notion-fetch", "notion-fetch"])
    }

    @Test func failedInitializationDoesNotLeaveAnOldSessionOnTheCachedClient() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef"
        OAuthFixtureProtocol.failNextInitialized = true
        OAuthFixtureProtocol.tools = [(200, page(parent))]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults())
        let shared = session()
        await #expect(throws: NotionExportError.self) { _ = try await connection.client(session: shared).parentTitle(id: parent) }
        _ = try await connection.client(session: shared).parentTitle(id: parent)
        let initialize = OAuthFixtureProtocol.requests.filter { String(data: $0.httpBody ?? Data(), encoding: .utf8)?.contains("\"method\":\"initialize\"") == true }
        #expect(initialize.count == 2)
        #expect(initialize.allSatisfy { $0.value(forHTTPHeaderField: "Mcp-Session-Id") == nil })
    }

    @Test func expiredSessionNeverAutomaticallyRepeatsAWritingRequest() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef"
        OAuthFixtureProtocol.tools = [(200, page(parent)), (404, [:]), (200, page(parent))]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults())
        let client = try connection.client(session: session())
        _ = try await client.parentTitle(id: parent)
        do { try await client.appendDivider(parent: parent); Issue.record("Expired write succeeded") }
        catch NotionExportError.sessionExpired { }
        catch { Issue.record("Unexpected expiry result: \(error)") }
        _ = try await client.parentTitle(id: parent)
        let tools = rpcCalls().compactMap { $0["params"] as? [String: Any] }.compactMap { $0["name"] as? String }
        #expect(tools.filter { $0 == "notion-update-page" }.count == 1)
        #expect(rpcCalls().filter { $0["method"] as? String == "initialize" }.count == 2)
    }

    @Test func asynchronousDividerUsesEveryServerPollHintAndRecordsWaitingSeparately() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef"
        OAuthFixtureProtocol.tools = [(200, ["object": "async_task", "id": "task", "status": "queued", "poll_after_seconds": 0.5]),
            (200, ["status": "running", "poll_after_seconds": 1.25]), (200, ["status": "succeeded", "result": ["page_id": parent]])]
        let storage = MemoryNotionConnectionStore(); let grant = try grant(); storage.value = .oauth(grant)
        let diagnostics = NotionDiagnostics()
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults(), diagnostics: diagnostics)
        var time: TimeInterval = 0, waits: [TimeInterval] = []
        let policy = NotionAsyncPolling(timeout: 10, now: { time }, wait: { waits.append($0); time += $0 })
        let client = NotionMCPPageClient(connection: connection, grantID: grant.id, session: session(), polling: policy)
        try await client.appendDivider(parent: parent)
        #expect(waits == [0.5, 1.25])
        #expect(diagnostics.entries.filter { $0.kind == .pollWait }.map(\.milliseconds) == [500, 1250])
        let tools = rpcCalls().compactMap { $0["params"] as? [String: Any] }.compactMap { $0["name"] as? String }
        #expect(tools == ["notion-update-page", "notion-get-async-task", "notion-get-async-task"])
    }

    @Test func completedAsyncResponseDoesNotSleepAndLongHintNeverCausesEarlyPolling() async throws {
        for completed in [true, false] {
            OAuthFixtureProtocol.reset()
            let parent = "01234567-89ab-cdef-0123-456789abcdef"
            OAuthFixtureProtocol.tools = [(200, completed ? ["object": "async_task", "id": "task", "status": "succeeded", "result": ["page_id": parent]]
                : ["object": "async_task", "id": "task", "status": "queued", "poll_after_seconds": 120])]
            let storage = MemoryNotionConnectionStore(); let grant = try grant(); storage.value = .oauth(grant)
            let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults())
            let policy = NotionAsyncPolling(timeout: 60, now: { 0 }, wait: { _ in Issue.record("Unexpected wait") })
            let client = NotionMCPPageClient(connection: connection, grantID: grant.id, session: session(), polling: policy)
            if completed { try await client.appendDivider(parent: parent) }
            else { await #expect(throws: NotionExportError.self) { try await client.appendDivider(parent: parent) } }
            #expect(rpcCalls().filter { $0["method"] as? String == "tools/call" }.count == 1)
        }
    }

    @Test func pollingHintsRejectMalformedNumbersWithoutUsingTheirValues() {
        for value: Any in [0, -1, true, "1", Double.infinity, Double.nan] {
            #expect(NotionAsyncPolling.interval(["poll_after_seconds": value], fallback: 3) == 3)
        }
        #expect(NotionAsyncPolling.interval(["poll_after_seconds": 1]) == 1)
    }

    @Test func expiredPollingBudgetPersistsTheDividerCheckpointAndNeverRepeatsItsWrite() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef"
        OAuthFixtureProtocol.tools = [(200, page(parent)), (200, ["object": "async_task", "id": "task", "status": "queued", "poll_after_seconds": 1])]
            + Array(repeating: (200, ["status": "running", "poll_after_seconds": 1]), count: 4) + [(200, page(parent))]
        let storage = MemoryNotionConnectionStore(); let grant = try grant(); storage.value = .oauth(grant)
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), defaults: defaults())
        var time: TimeInterval = 0
        let policy = NotionAsyncPolling(timeout: 5, now: { time }, wait: { time += $0 })
        let client = NotionMCPPageClient(connection: connection, grantID: grant.id, session: session(), polling: policy)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let meeting = MeetingRecord(title: "검증 회의", startedAt: Date(), duration: 1, status: .ready, glossaryProfile: "", transcript: [], claims: [])
        let document = try TranscriptDocument(speakers: [], utterances: [.init(id: UUID(), startTime: 0, endTime: 1,
            rawText: "원문", sourceChannelID: "recording", engineClusterID: nil, speakerID: nil, editedText: nil)])
        let exporter = NotionExporter(directory: directory)
        await #expect(throws: NotionExportError.self) { _ = try await exporter.save(meeting: meeting, document: document, parentLink: parent, original: false, client: client) }
        #expect(try exporter.receipt(meetingID: meeting.id)?.phase == .dividerPending)
        let reopened = NotionExporter(directory: directory)
        await #expect(throws: NotionExportError.self) { _ = try await reopened.save(meeting: meeting, document: document, parentLink: parent, original: false, client: client) }
        let names = rpcCalls().compactMap { $0["params"] as? [String: Any] }.compactMap { $0["name"] as? String }
        #expect(names.filter { $0 == "notion-update-page" }.count == 1)
        #expect(!names.contains("notion-create-pages"))
    }

    @Test func oauthExportAppendsADividerCreatesOneChildAndVerifiesItsParent() async throws {
        OAuthFixtureProtocol.reset()
        let parent = "01234567-89ab-cdef-0123-456789abcdef", child = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        OAuthFixtureProtocol.tools = [(200, page(parent)), (200, ["page_id": parent.replacingOccurrences(of: "-", with: "")]),
            (200, ["pages": [["id": child]]]), (200, page(child, parent: parent))]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let exporter = NotionExporter(directory: directory)
        let meeting = MeetingRecord(title: "검증 회의", startedAt: Date(), duration: 1, status: .ready, glossaryProfile: "", transcript: [], claims: [])
        let document = try TranscriptDocument(speakers: [], utterances: [.init(id: UUID(), startTime: 0, endTime: 1, rawText: "전체 원문", sourceChannelID: "recording", engineClusterID: nil, speakerID: nil, editedText: nil)])
        let url = try await exporter.save(meeting: meeting, document: document, parentLink: parent, original: false, client: connection.client(session: session()))
        #expect(url == NotionPageLink.url(for: child))
        #expect(try exporter.receipt(meetingID: meeting.id)?.verified == true)
        let calls = OAuthFixtureProtocol.requests.filter { $0.url?.path == "/mcp" }.compactMap { try? JSONSerialization.jsonObject(with: $0.httpBody ?? Data()) as? [String: Any] }
        let tools = calls.filter { $0["method"] as? String == "tools/call" }.compactMap { $0["params"] as? [String: Any] }
        #expect(tools.compactMap { $0["name"] as? String } == ["notion-fetch", "notion-update-page", "notion-create-pages", "notion-fetch"])
        let update = tools[1]["arguments"] as? [String: Any]
        #expect(update?["command"] as? String == "insert_content")
        #expect(update?["content"] as? String == "\n---\n")
        #expect(update?["new_str"] == nil)
        #expect((update?["position"] as? [String: Any])?["type"] as? String == "end")
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/mcp" }.dropFirst().allSatisfy { $0.value(forHTTPHeaderField: "Mcp-Session-Id") == "fixture-session" })
    }

    @Test func uncertainMCPPageCreationIsNotAutomaticallyRetried() async throws {
        OAuthFixtureProtocol.reset()
        OAuthFixtureProtocol.tools = [(200, ["object": "async_task", "id": "task-accepted"])]
        let storage = MemoryNotionConnectionStore(); storage.value = .oauth(try grant())
        let connection = NotionConnection(storage: storage, service: NotionOAuthService(session: session()), browser: OAuthFixtureBrowser(), defaults: defaults())
        let client = try connection.client(session: session())
        await #expect(throws: (any Error).self) { try await client.create(parent: "01234567-89ab-cdef-0123-456789abcdef", title: "회의록", markdown: "원문") }
        #expect(OAuthFixtureProtocol.requests.filter { $0.url?.path == "/mcp" && String(data: $0.httpBody ?? Data(), encoding: .utf8)?.contains("notion-create-pages") == true }.count == 1)
    }

    @Test func truncatedAndUnexpectedPageResponsesAreRejectedBeforeWriting() throws {
        let id = "01234567-89ab-cdef-0123-456789abcdef"
        var result = page(id); result["truncated"] = true
        #expect(throws: (any Error).self) { try NotionFetchedPage(response: result, expectedID: id) }
        #expect(throws: (any Error).self) { try NotionFetchedPage(response: ["text": "<database url=\"https://www.notion.so/0123456789abcdef0123456789abcdef\"></database>"], expectedID: id) }
        #expect(throws: (any Error).self) { try NotionFetchedPage(response: page(id), expectedID: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee") }
    }
}
