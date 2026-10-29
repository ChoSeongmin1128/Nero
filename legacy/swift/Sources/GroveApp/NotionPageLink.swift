import Foundation

enum NotionPageLink {
    static func id(from input: String) throws -> String {
        let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
        if let id = normalizedID(value) { return id }
        guard let components = URLComponents(string: value), let scheme = components.scheme?.lowercased() else {
            throw NotionExportError.invalidLink
        }
        if ["collection", "view", "discussion", "user"].contains(scheme) { throw NotionExportError.unsupportedParent }
        guard ["https", "notion"].contains(scheme), let host = components.host?.lowercased(),
              isNotionHost(host), components.user == nil, components.password == nil,
              components.port == nil || (scheme == "https" && components.port == 443) else {
            throw NotionExportError.invalidLink
        }
        let segment = components.path.split(separator: "/", omittingEmptySubsequences: true).last.map(String.init) ?? ""
        guard let id = pageID(in: segment) else { throw NotionExportError.pageLinkRequired }
        // View / tracking / block anchors do not replace the page. Undocumented peek targets must not silently select another page.
        for name in ["p", "peek"] {
            let targets = (components.queryItems ?? []).filter { $0.name == name }
            guard targets.isEmpty || (targets.count == 1 && normalizedID(targets[0].value ?? "") == id) else {
                throw NotionExportError.ambiguousLink
            }
        }
        return id
    }

    static func url(for id: String) -> URL {
        URL(string: "https://app.notion.com/p/" + id.replacingOccurrences(of: "-", with: ""))!
    }

    static func validationMessage(for input: String) -> String? {
        guard !input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        do { _ = try id(from: input); return nil }
        catch { return error.localizedDescription }
    }

    private static func isNotionHost(_ host: String) -> Bool {
        ["notion.com", "www.notion.com", "app.notion.com", "notion.so", "notion.site"].contains(host)
            || host.hasSuffix(".notion.so") || host.hasSuffix(".notion.site")
    }

    private static func pageID(in segment: String) -> String? {
        for count in [36, 32] where segment.count >= count {
            let start = segment.index(segment.endIndex, offsetBy: -count)
            guard start == segment.startIndex || segment[segment.index(before: start)] == "-" else { continue }
            if let id = normalizedID(String(segment[start...])) { return id }
        }
        return nil
    }

    private static func normalizedID(_ value: String) -> String? {
        guard value.range(of: #"\A(?:[0-9a-fA-F]{32}|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\z"#,
                          options: .regularExpression) != nil else { return nil }
        let hex = value.replacingOccurrences(of: "-", with: "").lowercased(), chars = Array(hex)
        return [String(chars[0..<8]), String(chars[8..<12]), String(chars[12..<16]), String(chars[16..<20]), String(chars[20..<32])].joined(separator: "-")
    }
}
