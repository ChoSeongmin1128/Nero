import Foundation

private enum NotionMCPSessionError: Error { case expired(String) }

@MainActor
final class NotionMCPTransport {
    private weak var connection: NotionConnection?
    private let grantID: UUID
    private let session: URLSession
    private var sessionID: String?
    private var protocolVersion = "2025-11-25"
    private var initialized = false
    private var initialization: Task<Void, any Error>?
    private let diagnostics: NotionDiagnostics

    init(connection: NotionConnection, grantID: UUID, session: URLSession) {
        self.connection = connection; self.grantID = grantID; self.session = session
        diagnostics = connection.diagnostics
    }

    func call(_ name: String, arguments: [String: Any], writes: Bool = false, timeout: TimeInterval = 60) async throws -> [String: Any] {
        let params: [String: Any] = ["name": name, "arguments": arguments]
        let result: [String: Any]
        do {
            try await initialize()
            result = try await rpc("tools/call", params: params, writes: writes, timeout: timeout)
        } catch NotionMCPSessionError.expired(let expiredID) {
            resetSession(ifMatching: expiredID)
            guard !writes else { throw NotionExportError.sessionExpired }
            do {
                try await initialize()
                result = try await rpc("tools/call", params: params, timeout: timeout)
            } catch NotionMCPSessionError.expired(let secondID) {
                resetSession(ifMatching: secondID)
                throw NotionExportError.http(404)
            }
        }
        if result["isError"] as? Bool == true {
            let envelope = result["structuredContent"] as? [String: Any]
                ?? (result["content"] as? [[String: Any]])?.compactMap { block -> [String: Any]? in
                    guard let text = block["text"] as? String, let data = text.data(using: .utf8) else { return nil }
                    return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
                }.first
            let error = envelope?["error"] as? [String: Any] ?? envelope
            if error?["code"] as? String == "validation_error" { throw NotionExportError.requestRejected }
            if writes { throw NotionExportError.unknownResult }
            throw NotionConnectionError.unavailable
        }
        if let structured = result["structuredContent"] as? [String: Any] { return structured }
        let texts = (result["content"] as? [[String: Any]] ?? []).compactMap { $0["type"] as? String == "text" ? $0["text"] as? String : nil }
        guard texts.count == 1 else { throw NotionExportError.unknownResult }
        if let data = texts[0].data(using: .utf8), let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] { return object }
        return ["text": texts[0]]
    }

    private func initialize() async throws {
        if initialized { return }
        if let initialization { return try await initialization.value }
        let task = Task { @MainActor in
            do {
                let result = try await rpc("initialize", params: ["protocolVersion": protocolVersion,
                    "capabilities": [:], "clientInfo": ["name": "Grove", "version": Self.clientVersion]])
                guard let version = result["protocolVersion"] as? String,
                      ["2025-03-26", "2025-06-18", "2025-11-25"].contains(version) else { throw NotionConnectionError.invalidResponse }
                protocolVersion = version
                _ = try await rpc("notifications/initialized", params: [:], notification: true)
                initialized = true
            } catch {
                sessionID = nil
                initialized = false
                throw error
            }
        }
        initialization = task
        defer { initialization = nil }
        try await task.value
    }

    private static let clientVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"

    private func resetSession(ifMatching id: String) {
        guard sessionID == id else { return }
        sessionID = nil
        initialized = false
    }

    private func rpc(_ method: String, params: [String: Any], writes: Bool = false, notification: Bool = false,
                     timeout: TimeInterval = 60) async throws -> [String: Any] {
        let id = UUID().uuidString
        var payload: [String: Any] = ["jsonrpc": "2.0", "method": method, "params": params]
        if !notification { payload["id"] = id }
        let body = try JSONSerialization.data(withJSONObject: payload)
        guard let connection else { throw NotionConnectionError.connectionChanged }
        var token = try await connection.accessToken(grantID: grantID)
        for attempt in 0...1 {
            let startedAt = Date(), started = ProcessInfo.processInfo.systemUptime
            var statusCode: Int?, responseBytes = 0, failed = true
            var responseElapsed: TimeInterval?
            defer {
                diagnostics.record(kind: .request, operation: params["name"] as? String ?? method,
                    startedAt: startedAt, elapsed: responseElapsed ?? ProcessInfo.processInfo.systemUptime - started,
                    statusCode: statusCode, requestBytes: body.count, responseBytes: responseBytes, failed: failed)
            }
            let requestSessionID = sessionID
            var request = URLRequest(url: NotionOAuthConfiguration.server)
            request.httpMethod = "POST"; request.httpBody = body; request.timeoutInterval = timeout
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.setValue("application/json, text/event-stream", forHTTPHeaderField: "Accept")
            request.setValue(protocolVersion, forHTTPHeaderField: "MCP-Protocol-Version")
            if let sessionID { request.setValue(sessionID, forHTTPHeaderField: "Mcp-Session-Id") }
            let data: Data, response: URLResponse
            do { (data, response) = try await session.data(for: request) }
            catch { if writes { throw NotionExportError.unknownResult }; throw error }
            responseElapsed = ProcessInfo.processInfo.systemUptime - started
            guard let http = response as? HTTPURLResponse else { throw NotionExportError.unknownResult }
            statusCode = http.statusCode
            responseBytes = data.count
            try connection.validateGrant(id: grantID)
            if http.statusCode == 401 && attempt == 0 {
                token = try await connection.accessToken(grantID: grantID, rejectedToken: token)
                continue
            }
            if http.statusCode == 404, let requestSessionID { throw NotionMCPSessionError.expired(requestSessionID) }
            guard (200..<300).contains(http.statusCode) else {
                if writes && http.statusCode >= 500 { throw NotionExportError.unknownResult }
                throw NotionExportError.http(http.statusCode)
            }
            if let value = http.value(forHTTPHeaderField: "Mcp-Session-Id"), requestSessionID == nil || requestSessionID == sessionID { sessionID = value }
            if notification { failed = false; return [:] }
            let result = try Self.result(data: data, contentType: http.value(forHTTPHeaderField: "Content-Type") ?? "", id: id)
            failed = result["isError"] as? Bool == true
            return result
        }
        throw NotionConnectionError.reconnectRequired
    }

    static func result(data: Data, contentType: String, id: String) throws -> [String: Any] {
        guard data.count <= 2 * 1024 * 1024 else { throw NotionExportError.unknownResult }
        let messages: [[String: Any]]
        if contentType.lowercased().contains("text/event-stream") {
            guard let text = String(data: data, encoding: .utf8) else { throw NotionExportError.unknownResult }
            messages = text.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n\n").compactMap { event in
                let payload = event.components(separatedBy: "\n").filter { $0.hasPrefix("data:") }
                    .map { String($0.dropFirst(5)).trimmingCharacters(in: .whitespaces) }.joined(separator: "\n")
                return payload.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
            }
        } else {
            guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NotionExportError.unknownResult }
            messages = [value]
        }
        let replies = messages.filter { $0["id"] as? String == id && $0["jsonrpc"] as? String == "2.0" }
        guard replies.count == 1 else { throw NotionExportError.unknownResult }
        if let error = replies[0]["error"] as? [String: Any],
           let code = error["code"] as? Int, [-32601, -32602].contains(code) {
            throw NotionExportError.requestRejected
        }
        guard replies[0]["error"] == nil,
              let result = replies[0]["result"] as? [String: Any] else { throw NotionExportError.unknownResult }
        return result
    }
}

struct NotionFetchedPage {
    let id: String
    let title: String
    let content: String
    let parentID: String?

    init(response: [String: Any], expectedID: String) throws {
        let kind = response["type"] as? String ?? (response["metadata"] as? [String: Any])?["type"] as? String
        if let kind, kind != "page" { throw NotionExportError.unsupportedParent }
        guard response["truncated"] as? Bool != true,
              (response["unknown_block_count"] as? Int ?? 0) == 0 else { throw NotionExportError.invalidParent }
        let text = response["text"] as? String ?? response["content"] as? String ?? ""
        let page: String
        if text.contains("<page ") { page = text }
        else {
            if text.contains("<database ") || text.contains("<data-source ") || text.contains("<view ") || text.contains("<folder ") { throw NotionExportError.unsupportedParent }
            throw NotionExportError.invalidParent
        }
        guard !page.contains("<unknown"), response["type"] as? String != "database",
              let opening = Self.capture(#"<page\s[^>]*url="([^"]+)"[^>]*>"#, in: page),
              let id = try? NotionPageLink.id(from: opening), id == expectedID,
              let properties = Self.capture(#"(?s)<properties>\s*(.*?)\s*</properties>"#, in: page),
              let data = properties.data(using: .utf8), let values = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let title = response["title"] as? String ?? values["title"] as? String,
              page.trimmingCharacters(in: .whitespacesAndNewlines).hasSuffix("</page>"),
              let content = Self.pageContent(in: page) else { throw NotionExportError.invalidParent }
        self.id = id; self.title = title; self.content = content
        let ancestors = Self.capture(#"(?s)<ancestor-path>(.*?)</ancestor-path>"#, in: page) ?? ""
        parentID = Self.capture(#"<parent-page\s[^>]*url="([^"]+)"[^>]*>"#, in: ancestors).flatMap { try? NotionPageLink.id(from: $0) }
    }

    private static func pageContent(in page: String) -> String? {
        let content = capture(#"(?s)<content>\n?(.*?)\n?</content>"#, in: page)
        let blank = capture(#"(?s)<blank-page>(.*?)</blank-page>"#, in: page)
        if let content { return blank == nil ? content : nil }
        guard blank != nil, !page.contains("<content>") else { return nil }
        return ""
    }

    private static func capture(_ pattern: String, in text: String) -> String? {
        guard let regex = try? NSRegularExpression(pattern: pattern),
              let match = regex.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)),
              let range = Range(match.range(at: 1), in: text) else { return nil }
        return String(text[range])
    }
}

@MainActor
final class NotionMCPPageClient: NotionPageClient {
    private let transport: NotionMCPTransport
    private let polling: NotionAsyncPolling
    private let diagnostics: NotionDiagnostics
    init(connection: NotionConnection, grantID: UUID, session: URLSession, polling: NotionAsyncPolling = NotionAsyncPolling()) {
        transport = NotionMCPTransport(connection: connection, grantID: grantID, session: session)
        self.polling = polling
        diagnostics = connection.diagnostics
    }

    private func fetch(_ id: String) async throws -> NotionFetchedPage {
        try await NotionFetchedPage(response: transport.call("notion-fetch", arguments: ["id": NotionPageLink.url(for: id).absoluteString]), expectedID: id)
    }
    func parentTitle(id: String) async throws -> String { try await fetch(id).title }

    func inspectParent(id: String) async throws -> NotionExportDestination {
        let page = try await fetch(id)
        return .init(id: page.id, hasDivider: Self.hasDivider(page.content))
    }

    func appendDivider(parent: String) async throws {
        do {
            let result = try await transport.call("notion-update-page", arguments: ["page_id": parent,
                "command": "insert_content", "content": "\n---\n", "position": ["type": "end"], "allow_async": false], writes: true)
            try await completeUpdate(result, expectedID: parent)
        } catch NotionExportError.requestRejected { throw NotionExportError.requestRejected }
        catch NotionExportError.sessionExpired { throw NotionExportError.sessionExpired }
        catch NotionExportError.http(let status) { throw NotionExportError.http(status) }
        catch { throw NotionExportError.dividerFailed }
    }
    func hasDividerAtEnd(parent: String) async throws -> Bool {
        let page = try await fetch(parent)
        return Self.hasDivider(page.content)
    }

    private static func hasDivider(_ content: String) -> Bool {
        content.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n").last == "---"
    }

    func create(parent: String, title: String, markdown: String) async throws -> String {
        let result = try await transport.call("notion-create-pages", arguments: ["parent": ["page_id": parent],
            "pages": [["properties": ["title": title], "content": markdown]], "allow_async": false], writes: true)
        guard result["object"] as? String != "async_task", Self.hasNoWarnings(result),
              let pages = result["pages"] as? [[String: Any]], pages.count == 1, Self.hasNoWarnings(pages[0]) else { throw NotionExportError.unknownResult }
        if let id = pages[0]["id"] as? String, let normalized = try? NotionPageLink.id(from: id) { return normalized }
        if let url = pages[0]["url"] as? String, let id = try? NotionPageLink.id(from: url) { return id }
        throw NotionExportError.unknownResult
    }

    func verifyChild(id: String, parent: String) async throws {
        guard try await fetch(id).parentID == parent else { throw NotionExportError.invalidParent }
    }

    private func completeUpdate(_ response: [String: Any], expectedID: String) async throws {
        var result = response
        if result["object"] as? String == "async_task" {
            guard let task = result["id"] as? String else { throw NotionExportError.unknownResult }
            let deadline = polling.now() + polling.timeout
            var interval = NotionAsyncPolling.interval(result)
            while true {
                if result["status"] as? String == "succeeded", let completed = result["result"] as? [String: Any] {
                    result = completed
                    break
                }
                if result["status"] as? String == "failed" { throw NotionExportError.dividerFailed }
                if let status = result["status"] as? String, !["queued", "running", "retrying"].contains(status) { throw NotionExportError.unknownResult }
                interval = NotionAsyncPolling.interval(result, fallback: interval)
                guard polling.now() + interval < deadline else { throw NotionExportError.unknownResult }
                let startedAt = Date(), started = polling.now()
                try await polling.wait(interval)
                diagnostics.record(kind: .pollWait, operation: "poll", startedAt: startedAt, elapsed: polling.now() - started)
                try Task.checkCancellation()
                let remaining = deadline - polling.now()
                guard remaining > 0 else { throw NotionExportError.unknownResult }
                result = try await transport.call("notion-get-async-task", arguments: ["task_id": task], timeout: remaining)
            }
        }
        guard let pageID = result["page_id"] as? String, (try? NotionPageLink.id(from: pageID)) == expectedID,
              Self.hasNoWarnings(result) else { throw NotionExportError.unknownResult }
    }
    private static func hasNoWarnings(_ value: [String: Any]) -> Bool {
        guard let warnings = value["warnings"] else { return true }
        return (warnings as? [Any])?.isEmpty == true
    }
}
