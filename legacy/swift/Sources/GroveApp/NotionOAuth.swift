import AuthenticationServices
import AppKit
import CryptoKit
import Foundation
import Security

enum NotionConnectionError: Error, LocalizedError {
    case invalidResponse, invalidCallback, cancelled, reconnectRequired, connectionChanged, unavailable

    var errorDescription: String? {
        switch self {
        case .invalidResponse: "Notion 연결 응답을 확인하지 못했습니다. 다시 시도해 주세요."
        case .invalidCallback: "Notion 로그인 결과가 현재 요청과 일치하지 않습니다."
        case .cancelled: "Notion 연결을 취소했습니다."
        case .reconnectRequired: "Notion에 다시 연결해 주세요."
        case .connectionChanged: "Notion 연결이 변경됐습니다. 저장 위치를 다시 확인해 주세요."
        case .unavailable: "현재 Notion 연결을 사용할 수 없습니다."
        }
    }
}

enum NotionOAuthConfiguration {
    static let server = URL(string: "https://mcp.notion.com/mcp")!
    static let issuer = "https://mcp.notion.com"
    static let redirect = URL(string: "io.github.choseongmin1128.grove://notion/callback")!

    static func trustedEndpoint(_ value: String) throws -> URL {
        guard let url = URL(string: value), url.scheme == "https", url.host == "mcp.notion.com",
              url.port == nil, url.user == nil, url.password == nil, url.fragment == nil else {
            throw NotionConnectionError.invalidResponse
        }
        return url
    }
}

struct NotionOAuthMetadata: Codable, Equatable {
    let issuer: String
    let authorizationEndpoint: String
    let tokenEndpoint: String
    let registrationEndpoint: String
    let revocationEndpoint: String?
    let codeChallengeMethodsSupported: [String]
    let tokenEndpointAuthMethodsSupported: [String]

    func validate() throws {
        guard issuer == NotionOAuthConfiguration.issuer, codeChallengeMethodsSupported.contains("S256"),
              tokenEndpointAuthMethodsSupported.contains("none") else { throw NotionConnectionError.invalidResponse }
        for endpoint in [authorizationEndpoint, tokenEndpoint, registrationEndpoint] { _ = try NotionOAuthConfiguration.trustedEndpoint(endpoint) }
        if let revocationEndpoint { _ = try NotionOAuthConfiguration.trustedEndpoint(revocationEndpoint) }
    }
}

struct NotionOAuthTokens: Decodable {
    let accessToken: String
    let tokenType: String
    let refreshToken: String?
    let expiresIn: Double
    let workspaceId: String?
    let userId: String?
}

struct NotionOAuthAttempt {
    let state: String
    let verifier: String
    let startedAt: Date

    init(state: String? = nil, verifier: String? = nil, startedAt: Date = Date()) throws {
        self.state = try state ?? Self.randomString()
        self.verifier = try verifier ?? Self.randomString()
        self.startedAt = startedAt
    }

    var challenge: String { Data(SHA256.hash(data: Data(verifier.utf8))).base64URLEncoded }

    func authorizationURL(metadata: NotionOAuthMetadata, clientID: String) throws -> URL {
        var components = URLComponents(url: try NotionOAuthConfiguration.trustedEndpoint(metadata.authorizationEndpoint), resolvingAgainstBaseURL: false)!
        components.queryItems = ["response_type": "code", "client_id": clientID,
            "redirect_uri": NotionOAuthConfiguration.redirect.absoluteString, "state": state,
            "code_challenge": challenge, "code_challenge_method": "S256", "scope": "default",
            "resource": NotionOAuthConfiguration.server.absoluteString].sorted { $0.key < $1.key }.map { URLQueryItem(name: $0.key, value: $0.value) }
        guard let url = components.url else { throw NotionConnectionError.invalidResponse }
        return url
    }

    func authorizationCode(from callback: URL, now: Date = Date()) throws -> String {
        guard now.timeIntervalSince(startedAt) >= 0, now.timeIntervalSince(startedAt) <= 600,
              callback.scheme?.lowercased() == NotionOAuthConfiguration.redirect.scheme,
              callback.host == "notion", callback.path == "/callback", callback.port == nil,
              callback.user == nil, callback.password == nil, callback.fragment == nil,
              let items = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems,
              Set(items.map(\.name)).count == items.count else { throw NotionConnectionError.invalidCallback }
        let values = Dictionary(uniqueKeysWithValues: items.map { ($0.name, $0.value ?? "") })
        guard values["state"] == state,
              values["iss"].map({ $0 == NotionOAuthConfiguration.issuer }) ?? true else { throw NotionConnectionError.invalidCallback }
        if values["error"] == "access_denied" { throw NotionConnectionError.cancelled }
        guard values["error"] == nil, let code = values["code"], !code.isEmpty, code.count <= 2048 else { throw NotionConnectionError.invalidCallback }
        return code
    }

    private static func randomString() throws -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { throw NotionConnectionError.unavailable }
        return Data(bytes).base64URLEncoded
    }
}

private extension Data {
    var base64URLEncoded: String { base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "") }
}

final class NotionNoRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) { completionHandler(nil) }

    static func session() -> URLSession { URLSession(configuration: .ephemeral, delegate: NotionNoRedirects(), delegateQueue: nil) }
}

@MainActor
protocol NotionBrowserAuthorizing {
    func authorize(url: URL) async throws -> URL
    func cancel()
}

@MainActor
protocol NotionAuthenticationSession {
    func start(presenting context: any ASWebAuthenticationPresentationContextProviding) -> Bool
    func cancel()
}

@MainActor
private final class SystemNotionAuthenticationSession: NotionAuthenticationSession {
    private let session: ASWebAuthenticationSession

    init(url: URL, completion: @escaping @Sendable (URL?, (any Error)?) -> Void) {
        session = ASWebAuthenticationSession(url: url, callbackURLScheme: NotionOAuthConfiguration.redirect.scheme, completionHandler: completion)
    }

    func start(presenting context: any ASWebAuthenticationPresentationContextProviding) -> Bool {
        session.presentationContextProvider = context
        return session.start()
    }

    func cancel() { session.cancel() }
}

@MainActor
final class NotionBrowserAuthorization: NSObject, NotionBrowserAuthorizing, ASWebAuthenticationPresentationContextProviding {
    private let makeSession: (URL, @escaping @Sendable (URL?, (any Error)?) -> Void) -> any NotionAuthenticationSession
    private var session: (any NotionAuthenticationSession)?
    private var continuation: CheckedContinuation<URL, any Error>?
    private var attemptID: UUID?

    init(makeSession: @escaping (URL, @escaping @Sendable (URL?, (any Error)?) -> Void) -> any NotionAuthenticationSession = {
        SystemNotionAuthenticationSession(url: $0, completion: $1)
    }) {
        self.makeSession = makeSession
        super.init()
    }

    func authorize(url: URL) async throws -> URL {
        let id = UUID()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                self.attemptID = id
                self.continuation = continuation
                // AuthenticationServices can invoke this on its XPC queue, outside MainActor.
                let completion: @Sendable (URL?, (any Error)?) -> Void = { [weak self] callback, error in
                    Task { @MainActor in
                        let result: Result<URL, any Error>
                        if let callback { result = .success(callback) }
                        else if let error {
                            let cancelled = (error as NSError).domain == ASWebAuthenticationSessionErrorDomain
                                && (error as NSError).code == ASWebAuthenticationSessionError.canceledLogin.rawValue
                            result = .failure(cancelled ? NotionConnectionError.cancelled : error)
                        } else { result = .failure(NotionConnectionError.invalidResponse) }
                        self?.finish(result, id: id)
                    }
                }
                let session = makeSession(url, completion)
                self.session = session
                if !session.start(presenting: self) { finish(.failure(NotionConnectionError.unavailable), id: id) }
            }
        } onCancel: { Task { @MainActor in self.cancel(id: id) } }
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        NSApp.keyWindow ?? NSApp.windows.first(where: { $0.isVisible }) ?? ASPresentationAnchor()
    }

    func cancel() {
        guard let id = attemptID else { return }
        cancel(id: id)
    }
    private func cancel(id: UUID) {
        guard attemptID == id else { return }
        session?.cancel(); finish(.failure(NotionConnectionError.cancelled), id: id)
    }
    private func finish(_ result: Result<URL, any Error>, id: UUID) {
        guard attemptID == id else { return }
        let pending = continuation
        attemptID = nil
        continuation = nil
        session = nil
        pending?.resume(with: result)
    }
}

@MainActor
struct NotionOAuthService {
    var session: URLSession = NotionNoRedirects.session()

    func discover() async throws -> NotionOAuthMetadata {
        let resource = try await json(URL(string: "https://mcp.notion.com/.well-known/oauth-protected-resource/mcp")!)
        guard resource["resource"] as? String == NotionOAuthConfiguration.server.absoluteString,
              (resource["authorization_servers"] as? [String])?.contains(NotionOAuthConfiguration.issuer) == true else { throw NotionConnectionError.invalidResponse }
        let value = try await json(URL(string: "https://mcp.notion.com/.well-known/oauth-authorization-server")!)
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let metadata = try decoder.decode(NotionOAuthMetadata.self, from: JSONSerialization.data(withJSONObject: value))
        try metadata.validate()
        return metadata
    }

    func register(metadata: NotionOAuthMetadata) async throws -> String {
        let value = try await json(try NotionOAuthConfiguration.trustedEndpoint(metadata.registrationEndpoint), body: [
            "client_name": "Grove", "redirect_uris": [NotionOAuthConfiguration.redirect.absoluteString],
            "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"], "token_endpoint_auth_method": "none"])
        guard let id = value["client_id"] as? String, !id.isEmpty,
              value["token_endpoint_auth_method"] as? String == "none" else { throw NotionConnectionError.invalidResponse }
        return id
    }

    func exchange(code: String, attempt: NotionOAuthAttempt, clientID: String, metadata: NotionOAuthMetadata) async throws -> NotionOAuthTokens {
        try await tokens(fields: ["grant_type": "authorization_code", "code": code, "code_verifier": attempt.verifier,
            "client_id": clientID, "redirect_uri": NotionOAuthConfiguration.redirect.absoluteString,
            "resource": NotionOAuthConfiguration.server.absoluteString], metadata: metadata)
    }

    func refresh(_ grant: NotionOAuthGrant) async throws -> NotionOAuthTokens {
        try await tokens(fields: ["grant_type": "refresh_token", "refresh_token": grant.refreshToken,
            "client_id": grant.clientID, "resource": NotionOAuthConfiguration.server.absoluteString], metadata: grant.metadata)
    }

    private func tokens(fields: [String: String], metadata: NotionOAuthMetadata) async throws -> NotionOAuthTokens {
        var request = URLRequest(url: try NotionOAuthConfiguration.trustedEndpoint(metadata.tokenEndpoint))
        request.httpMethod = "POST"; request.timeoutInterval = 30
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        request.httpBody = Data(fields.sorted { $0.key < $1.key }.map {
            $0.key + "=" + $0.value.addingPercentEncoding(withAllowedCharacters: allowed)!
        }.joined(separator: "&").utf8)
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw NotionConnectionError.invalidResponse }
        if http.statusCode == 400, (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String == "invalid_grant" { throw NotionConnectionError.reconnectRequired }
        guard http.statusCode == 200, data.count <= 64 * 1024 else { throw NotionConnectionError.unavailable }
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        let value = try decoder.decode(NotionOAuthTokens.self, from: data)
        guard value.tokenType.lowercased() == "bearer", !value.accessToken.isEmpty,
              value.refreshToken?.isEmpty == false, value.expiresIn.isFinite, value.expiresIn > 0 else { throw NotionConnectionError.invalidResponse }
        return value
    }

    private func json(_ url: URL, body: [String: Any]? = nil) async throws -> [String: Any] {
        var request = URLRequest(url: url); request.timeoutInterval = 30
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if let body {
            request.httpMethod = "POST"; request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode), data.count <= 64 * 1024,
              let value = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NotionConnectionError.unavailable }
        return value
    }
}
