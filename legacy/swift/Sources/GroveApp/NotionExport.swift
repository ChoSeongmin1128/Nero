import AppKit
import CryptoKit
import Foundation
import Security

enum MeetingExportContent {
    static func title(_ meeting: MeetingRecord) -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd HH:mm"
        return "\(formatter.string(from: meeting.startedAt)) \(meeting.title)"
    }
    static func markdown(meeting: MeetingRecord, document: TranscriptDocument, original: Bool) -> String {
        let options = TranscriptExportOptions(format: .markdown, usesOriginalText: original)
        return "# \(TranscriptRenderer.escapeMarkdown(title(meeting)))\n\n"
            + (original ? "최초 전사문" : "검토한 전사문") + "\n\n"
            + TranscriptRenderer.render(document, options: options)
    }
    static func html(meeting: MeetingRecord, document: TranscriptDocument, original: Bool) -> String {
        let document = TranscriptRenderer.exportDocument(document, original: original)
        let body = document.utterances.sorted { $0.startTime < $1.startTime }.map { utterance in
            let heading = "[\(TranscriptRenderer.timestamp(utterance.startTime))] \(TranscriptRenderer.speakerLabel(in: document, for: utterance))"
            return "<p><b>\(escapeHTML(heading))</b><br>\(escapeHTML(original ? utterance.rawText : utterance.displayedText))</p>"
        }.joined()
        return "<html><head><meta charset=\"utf-8\"></head><body><h1>\(escapeHTML(title(meeting)))</h1>\(body)</body></html>"
    }
    static func copy(meeting: MeetingRecord, document: TranscriptDocument, original: Bool) -> Bool {
        let item = NSPasteboardItem()
        item.setString(markdown(meeting: meeting, document: document, original: original), forType: .string)
        item.setString(html(meeting: meeting, document: document, original: original), forType: .html)
        NSPasteboard.general.clearContents()
        return NSPasteboard.general.writeObjects([item])
    }
    private static func escapeHTML(_ value: String) -> String {
        value.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "\n", with: "<br>")
    }
}

@MainActor
protocol NotionPageClient {
    func parentTitle(id: String) async throws -> String
    func inspectParent(id: String) async throws -> NotionExportDestination
    func create(parent: String, title: String, markdown: String) async throws -> String
    func appendDivider(parent: String) async throws
    func hasDividerAtEnd(parent: String) async throws -> Bool
    func verifyChild(id: String, parent: String) async throws
}

struct NotionExportDestination {
    let id: String
    let hasDivider: Bool
}

extension NotionPageClient {
    func inspectParent(id: String) async throws -> NotionExportDestination {
        _ = try await parentTitle(id: id)
        return .init(id: id, hasDivider: try await hasDividerAtEnd(parent: id))
    }
}

@MainActor
struct NotionClient: NotionPageClient {
    let token: String
    var session: URLSession = .shared
    func parentTitle(id: String) async throws -> String {
        let page = try await request("pages/\(id)")
        guard page["object"] as? String == "page" else { throw NotionExportError.unsupportedParent }
        guard let responseID = page["id"] as? String, (try? NotionPageLink.id(from: responseID)) == id,
              page["in_trash"] as? Bool != true, page["archived"] as? Bool != true else {
            throw NotionExportError.invalidParent
        }
        let properties = page["properties"] as? [String: [String: Any]] ?? [:]
        return properties.values.compactMap { $0["title"] as? [[String: Any]] }.first?.compactMap { $0["plain_text"] as? String }.joined() ?? "Notion 페이지"
    }
    func create(parent: String, title: String, markdown: String) async throws -> String {
        let page = try await request("pages", method: "POST", body: ["parent": ["page_id": parent],
            "properties": ["title": ["title": [["text": ["content": title]]]]], "markdown": markdown])
        guard page["object"] as? String == "page", let id = page["id"] as? String,
              UUID(uuidString: id) != nil else { throw NotionExportError.unknownResult }
        return id
    }
    func appendDivider(parent: String) async throws {
        do {
            _ = try await request("pages/\(parent)/markdown", method: "PATCH",
                body: ["type": "insert_content", "insert_content": ["content": "\n---\n", "position": ["type": "end"]]])
        } catch NotionExportError.http(let status) { throw NotionExportError.http(status) }
        catch { throw NotionExportError.dividerFailed }
    }
    func hasDividerAtEnd(parent: String) async throws -> Bool {
        let page = try await request("pages/\(parent)/markdown")
        guard page["truncated"] as? Bool != true, let markdown = page["markdown"] as? String else {
            throw NotionExportError.invalidParent
        }
        return markdown.trimmingCharacters(in: .whitespacesAndNewlines).components(separatedBy: "\n").last == "---"
    }
    func verifyChild(id: String, parent: String) async throws {
        let page = try await request("pages/\(id)")
        let actual = (page["parent"] as? [String: Any])?["page_id"] as? String
        guard actual?.replacingOccurrences(of: "-", with: "").lowercased() == parent.replacingOccurrences(of: "-", with: "").lowercased(),
              page["in_trash"] as? Bool != true else { throw NotionExportError.invalidParent }
    }
    private func request(_ path: String, method: String = "GET", body: [String: Any]? = nil) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: "https://api.notion.com/v1/\(path)")!)
        request.httpMethod = method
        request.timeoutInterval = 60
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        request.setValue("2026-03-11", forHTTPHeaderField: "Notion-Version")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try body.map { try JSONSerialization.data(withJSONObject: $0) }
        let data: Data, response: URLResponse
        do { (data, response) = try await session.data(for: request) }
        catch { if method == "POST" { throw NotionExportError.unknownResult }; throw error }
        guard let http = response as? HTTPURLResponse else { throw NotionExportError.unknownResult }
        guard (200..<300).contains(http.statusCode) else {
            if method == "POST", http.statusCode >= 500 { throw NotionExportError.unknownResult }
            throw NotionExportError.http(http.statusCode)
        }
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw NotionExportError.unknownResult }
        return object
    }
}

struct NotionExportReceipt: Codable {
    enum Phase: String, Codable { case dividerPending, creationPending }
    let meetingID: UUID
    let parentID: String
    let contentHash: String
    var pageID: String?
    var verified = false
    var phase: Phase? = nil
}

enum NotionExportStage: Equatable {
    case checkingDestination, addingDivider, creatingNote, verifyingSave
    var label: String {
        switch self {
        case .checkingDestination: "저장 위치 확인 중"
        case .addingDivider: "구분선 추가 중"
        case .creatingNote: "회의록 저장 중"
        case .verifyingSave: "저장 결과 확인 중"
        }
    }
}

@MainActor
final class NotionExporter: ObservableObject {
    @Published private(set) var isSaving = false
    @Published private(set) var stage: NotionExportStage?
    @Published private(set) var message: String?
    let directory: URL
    private let diagnostics: NotionDiagnostics
    init(directory: URL, diagnostics: NotionDiagnostics = NotionDiagnostics()) {
        self.directory = directory
        self.diagnostics = diagnostics
    }
    func receipt(meetingID: UUID) throws -> NotionExportReceipt? {
        let url = directory.appendingPathComponent(meetingID.uuidString + ".json")
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        return try JSONDecoder().decode(NotionExportReceipt.self, from: Data(contentsOf: url))
    }
    func save(meeting: MeetingRecord, document: TranscriptDocument, parentLink: String, original: Bool,
              client: any NotionPageClient) async throws -> URL {
        guard !isSaving else { throw NotionExportError.busy }
        isSaving = true
        let startedAt = Date(), started = ProcessInfo.processInfo.systemUptime
        var succeeded = false
        defer {
            isSaving = false
            stage = nil
            diagnostics.record(kind: .export, operation: "save", startedAt: startedAt,
                elapsed: ProcessInfo.processInfo.systemUptime - started, failed: !succeeded)
        }
        let parent = try NotionPageLink.id(from: parentLink)
        let markdown = MeetingExportContent.markdown(meeting: meeting, document: document, original: original)
        let hash = SHA256.hash(data: Data(markdown.utf8)).map { String(format: "%02x", $0) }.joined()
        var receipt: NotionExportReceipt
        if let previous = try self.receipt(meetingID: meeting.id) {
            guard previous.parentID == parent, previous.contentHash == hash else { throw NotionExportError.changedExport }
            if let page = previous.pageID {
                stage = .verifyingSave
                try await client.verifyChild(id: page, parent: parent)
                succeeded = true
                return NotionPageLink.url(for: page)
            }
            guard previous.phase == .dividerPending else { throw NotionExportError.unknownResult }
            stage = .checkingDestination
            let destination = try await client.inspectParent(id: parent)
            guard destination.id == parent else { throw NotionExportError.invalidParent }
            guard destination.hasDivider else { throw NotionExportError.dividerPending }
            receipt = previous
        } else {
            stage = .checkingDestination
            let destination = try await client.inspectParent(id: parent)
            guard destination.id == parent else { throw NotionExportError.invalidParent }
            receipt = NotionExportReceipt(meetingID: meeting.id, parentID: parent, contentHash: hash, phase: .dividerPending)
            try persist(receipt)
            do {
                if !destination.hasDivider {
                    stage = .addingDivider
                    try await client.appendDivider(parent: parent)
                }
            }
            catch {
                if Self.isDefinitiveRejection(error) {
                    try? FileManager.default.removeItem(at: directory.appendingPathComponent(meeting.id.uuidString + ".json"))
                    throw error
                }
                throw NotionExportError.dividerPending
            }
        }
        receipt.phase = .creationPending
        try persist(receipt)
        let id: String
        stage = .creatingNote
        do { id = try await client.create(parent: parent, title: MeetingExportContent.title(meeting), markdown: markdown) }
        catch {
            if Self.isDefinitiveRejection(error) {
                try? FileManager.default.removeItem(at: directory.appendingPathComponent(meeting.id.uuidString + ".json"))
            }
            throw error
        }
        receipt.pageID = id
        try persist(receipt)
        stage = .verifyingSave
        try await client.verifyChild(id: id, parent: parent)
        receipt.verified = true
        try persist(receipt)
        succeeded = true
        return NotionPageLink.url(for: id)
    }
    func connectCreatedPage(meetingID: UUID, link: String, client: any NotionPageClient) async throws -> URL {
        guard !isSaving, var receipt = try receipt(meetingID: meetingID), receipt.pageID == nil,
              receipt.phase != .dividerPending else { throw NotionExportError.invalidParent }
        isSaving = true
        defer { isSaving = false }
        let page = try NotionPageLink.id(from: link)
        try await client.verifyChild(id: page, parent: receipt.parentID)
        receipt.pageID = page
        receipt.verified = true
        try persist(receipt)
        return NotionPageLink.url(for: page)
    }
    /// Clears a receipt whose divider request never got a confirmed result. Only the user
    /// does this after checking the page; no page creation was attempted for this receipt.
    func discardPendingDivider(meetingID: UUID) throws {
        guard !isSaving, let receipt = try receipt(meetingID: meetingID),
              receipt.phase == .dividerPending, receipt.pageID == nil else { throw NotionExportError.invalidParent }
        try FileManager.default.removeItem(at: directory.appendingPathComponent(meetingID.uuidString + ".json"))
    }

    private func persist(_ receipt: NotionExportReceipt) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try JSONEncoder().encode(receipt).write(to: directory.appendingPathComponent(receipt.meetingID.uuidString + ".json"), options: .atomic)
    }
    private static func isDefinitiveRejection(_ error: any Error) -> Bool {
        if case NotionExportError.requestRejected = error { return true }
        if case NotionExportError.sessionExpired = error { return true }
        if case NotionExportError.http(let status) = error { return (400..<500).contains(status) }
        return false
    }
}

enum NotionExportError: Error, LocalizedError {
    case invalidLink, pageLinkRequired, ambiguousLink, unsupportedParent, invalidParent, missingToken, keychain, unknownResult, changedExport, busy, dividerFailed, dividerPending, requestRejected, sessionExpired, http(Int)
    var errorDescription: String? {
        switch self {
        case .invalidLink: "Notion 페이지의 공유 링크를 입력해 주세요."
        case .pageLinkRequired: "저장할 페이지를 확인할 수 없습니다. Notion에서 페이지를 열고 공유 링크를 복사해 주세요."
        case .ambiguousLink: "저장할 페이지가 명확하지 않습니다. 해당 페이지를 전체 화면으로 열고 공유 링크를 복사해 주세요."
        case .unsupportedParent: "데이터베이스, 보기, 데이터 소스는 저장 위치로 사용할 수 없습니다. 추가할 페이지의 링크를 입력해 주세요."
        case .invalidParent: "저장할 페이지와 접근 권한을 확인해 주세요."
        case .missingToken: "설정에서 Notion에 연결해 주세요."
        case .keychain: "Notion 연결 정보를 Keychain에서 읽거나 저장하지 못했습니다."
        case .unknownResult: "저장 결과를 확인하지 못했습니다. 중복 생성을 막기 위해 다시 만들지 않습니다. Notion에서 생성된 페이지를 확인하고 아래에 링크를 연결해 주세요."
        case .changedExport: "이 녹음의 회의록은 이미 추가되었습니다. 추가된 회의록을 열거나 새 내용을 복사해 붙여넣어 주세요."
        case .busy: "다른 회의록을 저장하고 있습니다."
        case .dividerFailed: "구분선 추가 결과를 확인하지 못했습니다. 다시 적용하면 페이지 하단을 확인한 뒤 이어서 처리합니다."
        case .requestRejected: "Notion이 요청 형식을 거부했습니다. Grove 업데이트 후 다시 시도해 주세요."
        case .sessionExpired: "Notion 연결이 만료됐습니다. 다시 적용해 주세요."
        case .dividerPending: "구분선 추가 결과를 아직 확인하지 못했습니다. '결과 확인'을 누르거나, 페이지 끝에 구분선이 없다면 '처음부터 다시'를 눌러 주세요."
        case .http(let status): switch status {
            case 401: "Notion에 다시 연결해 주세요."
            case 403, 404: "이 페이지에 접근할 수 없습니다. Notion 페이지의 연결 설정과 쓰기 권한을 확인해 주세요."
            case 429: "Notion 요청이 잠시 제한되었습니다. 잠시 후 다시 시도해 주세요."
            default: "Notion 저장 요청에 실패했습니다 (\(status)). 원문은 Grove에 보존됩니다."
            }
        }
    }
}
