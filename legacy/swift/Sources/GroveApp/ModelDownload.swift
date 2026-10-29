import Foundation

final class ModelDownload: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let asset: ModelAsset
    private let partial: URL
    private let progress: @Sendable (Int64) -> Void
    private let lock = NSLock()
    private var task: URLSessionDataTask?
    private var cancelled = false
    private var continuation: CheckedContinuation<URL, any Error>?
    private var handle: FileHandle?
    private var received: Int64 = 0
    private var failure: (any Error)?
    private var session: URLSession?
    private let configuration: URLSessionConfiguration

    init(asset: ModelAsset, directory: URL, configuration: URLSessionConfiguration = .ephemeral,
         progress: @escaping @Sendable (Int64) -> Void) {
        self.asset = asset
        self.partial = directory.appendingPathComponent(asset.sha256 + ".partial")
        self.progress = progress
        self.configuration = configuration
    }

    func run() async throws -> URL {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                self.continuation = continuation
                do {
                    try FileManager.default.createDirectory(at: partial.deletingLastPathComponent(), withIntermediateDirectories: true)
                    if !FileManager.default.fileExists(atPath: partial.path) { FileManager.default.createFile(atPath: partial.path, contents: nil) }
                    let size = try partial.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
                    received = Int64(size)
                    handle = try FileHandle(forWritingTo: partial)
                    if received > asset.bytes { try handle?.truncate(atOffset: 0); received = 0 }
                    try handle?.seekToEnd()
                    var request = URLRequest(url: asset.remoteURL)
                    request.timeoutInterval = 120
                    request.setValue("identity", forHTTPHeaderField: "Accept-Encoding")
                    if received > 0 { request.setValue("bytes=\(received)-", forHTTPHeaderField: "Range") }
                    let queue = OperationQueue()
                    queue.maxConcurrentOperationCount = 1
                    queue.qualityOfService = .utility
                    configuration.timeoutIntervalForResource = 3600
                    let session = URLSession(configuration: configuration, delegate: self, delegateQueue: queue)
                    self.session = session
                    let task = session.dataTask(with: request)
                    lock.withLock {
                        self.task = task
                        if cancelled { task.cancel() }
                        task.resume()
                    }
                } catch { finish(error) }
            }
        } onCancel: {
            self.cancel()
        }
    }

    func cancel() { lock.withLock { cancelled = true; task?.cancel() } }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping @Sendable (URLSession.ResponseDisposition) -> Void) {
        do {
            guard let response = response as? HTTPURLResponse else { throw ModelPreparationError.download }
            if response.statusCode == 206 {
                guard response.value(forHTTPHeaderField: "Content-Range")?.hasPrefix("bytes \(received)-") == true else {
                    throw ModelPreparationError.integrity
                }
            } else if response.statusCode == 200 {
                try handle?.truncate(atOffset: 0)
                try handle?.seek(toOffset: 0)
                received = 0
            } else { throw ModelPreparationError.download }
            completionHandler(.allow)
        } catch { failure = error; completionHandler(.cancel) }
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        do {
            guard received + Int64(data.count) <= asset.bytes else { throw ModelPreparationError.integrity }
            try handle?.write(contentsOf: data)
            received += Int64(data.count)
            progress(received)
        } catch { failure = error; dataTask.cancel() }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        finish(failure ?? (lock.withLock { cancelled } ? CancellationError() : error))
    }

    private func finish(_ error: (any Error)?) {
        try? handle?.synchronize()
        try? handle?.close()
        handle = nil
        let continuation = self.continuation
        self.continuation = nil
        session?.finishTasksAndInvalidate()
        session = nil
        lock.withLock { task = nil }
        if let error { continuation?.resume(throwing: error) }
        else if received != asset.bytes { continuation?.resume(throwing: ModelPreparationError.integrity) }
        else { continuation?.resume(returning: partial) }
    }
}

// Download callbacks arrive per network chunk; publishing each one would redraw the UI hundreds of times a second.
final class ProgressThrottle: @unchecked Sendable {
    private let lock = NSLock()
    private let interval: TimeInterval
    private var last: TimeInterval = 0

    init(interval: TimeInterval = 0.2) { self.interval = interval }

    func shouldReport(_ bytes: Int64, of total: Int64) -> Bool {
        let now = ProcessInfo.processInfo.systemUptime
        return lock.withLock {
            guard bytes >= total || now - last >= interval else { return false }
            last = now
            return true
        }
    }
}
