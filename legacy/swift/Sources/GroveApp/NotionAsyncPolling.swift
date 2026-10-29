import CoreFoundation
import Foundation

@MainActor
struct NotionAsyncPolling {
    let timeout: TimeInterval
    let now: () -> TimeInterval
    let wait: (TimeInterval) async throws -> Void

    init(timeout: TimeInterval = 60, now: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime },
         wait: @escaping (TimeInterval) async throws -> Void = { try await Task.sleep(for: .seconds($0)) }) {
        self.timeout = timeout
        self.now = now
        self.wait = wait
    }

    static func interval(_ response: [String: Any], fallback: TimeInterval = 2) -> TimeInterval {
        guard let value = response["poll_after_seconds"] as? NSNumber,
              CFGetTypeID(value) != CFBooleanGetTypeID(), value.doubleValue.isFinite, value.doubleValue > 0 else { return fallback }
        return value.doubleValue
    }
}
