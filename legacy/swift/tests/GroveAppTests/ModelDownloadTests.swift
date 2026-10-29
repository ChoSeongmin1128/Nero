import Foundation
import Testing
@testable import GroveApp

private final class ModelProtocol: URLProtocol, @unchecked Sendable {
    nonisolated(unsafe) static var handler: (@Sendable (URLRequest, ModelProtocol) -> Void)?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() { Self.handler?(request, self) }
    override func stopLoading() {}
    func deliver(status: Int, headers: [String: String], data: String, fail: Bool = false) {
        client?.urlProtocol(self, didReceive: HTTPURLResponse(url: request.url!, statusCode: status,
            httpVersion: "HTTP/1.1", headerFields: headers)!, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(data.utf8))
        if fail {
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.05) { [self] in
                client?.urlProtocol(self, didFailWithError: URLError(.networkConnectionLost))
            }
        }
        else { client?.urlProtocolDidFinishLoading(self) }
    }
}

@Suite(.serialized)
struct ModelDownloadTests {
    @Test(.enabled(if: ProcessInfo.processInfo.environment["GROVE_RUN_DOWNLOAD_INTEGRATION"] == "1"))
    func realOfficialAssetCanBePausedAndResumedWithMatchingChecksum() async throws {
        let asset = try #require(ModelCatalog.assets.first { $0.name == "tokenizer.json" })
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let box = DownloadCancellationBox()
        let operation = ModelDownload(asset: asset, directory: directory) { bytes in
            if bytes >= 256 * 1024 { box.cancel() }
        }
        box.operation = operation
        do { _ = try await operation.run(); Issue.record("중단 요청이 반영되지 않음") }
        catch is CancellationError { }
        let partial = directory.appendingPathComponent(asset.sha256 + ".partial")
        let size = try partial.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        #expect(size >= 256 * 1024 && size < asset.bytes)
        let resumed = try await ModelDownload(asset: asset, directory: directory, progress: { _ in }).run()
        #expect(try await ModelManager.matches(resumed, asset: asset))
    }
    private var asset: ModelAsset { .init(group: .moss, repository: "synthetic", revision: "synthetic", name: "test.bin",
        bytes: 6, sha256: "bef57ec7f53a6d40beb640a780a639c83bc29ac8a9816f1fc6c5c6dcd93c4721") }
    private func configuration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [ModelProtocol.self]
        return configuration
    }
    @Test func serverIgnoringRangeRestartsWithoutDuplicatingOldBytes() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("abc".utf8).write(to: directory.appendingPathComponent(asset.sha256 + ".partial"))
        ModelProtocol.handler = { _, operation in operation.deliver(status: 200, headers: ["Content-Length": "6"], data: "abcdef") }
        let completed = try await ModelDownload(asset: asset, directory: directory, configuration: configuration(), progress: { _ in }).run()
        #expect(try Data(contentsOf: completed) == Data("abcdef".utf8))
    }
    @Test func wrongRangeDoesNotCorruptPreservedPartialFile() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let partial = directory.appendingPathComponent(asset.sha256 + ".partial")
        try Data("abc".utf8).write(to: partial)
        ModelProtocol.handler = { _, operation in operation.deliver(status: 206, headers: ["Content-Range": "bytes 2-5/6"], data: "cdef") }
        do { _ = try await ModelDownload(asset: asset, directory: directory, configuration: configuration(), progress: { _ in }).run(); Issue.record("잘못된 이어받기 범위가 허용됨") }
        catch { }
        #expect(try Data(contentsOf: partial) == Data("abc".utf8))
    }
}

private final class DownloadCancellationBox: @unchecked Sendable {
    weak var operation: ModelDownload?
    func cancel() { operation?.cancel() }
}
