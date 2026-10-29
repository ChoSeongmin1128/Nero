import CryptoKit
import Foundation
import GroveInference

struct ModelFileStamp: Codable, Equatable {
    let bytes: Int
    let modifiedMilliseconds: Int64
}

struct ModelReadinessReceipt: Codable {
    let runtime: String
    let files: [String: ModelFileStamp]
}

@MainActor
final class ModelManager: ObservableObject {
    @Published private(set) var readyGroups: Set<ModelGroup> = []
    @Published private(set) var isReadyForUse = false
    @Published private(set) var isVoiceModelReady = false
    @Published private(set) var isInstalling = false
    @Published private(set) var message: String?
    @Published private(set) var fraction: Double = 0
    @Published private(set) var downloadedBytes: Int64 = 0
    @Published private(set) var totalBytes: Int64 = 0
    let baseDirectory: URL
    private var task: Task<Void, Never>?
    private let runtimeID = "moss-\(MossModel.revision.prefix(8))-\(MossModel.workerRuntime)-nemotron-\(NemotronModel.revision.prefix(8))-\(NemotronModel.workerRuntime)"
    private var downloads: URL { baseDirectory.appendingPathComponent("Downloads", isDirectory: true) }
    private var receiptURL: URL { baseDirectory.appendingPathComponent("Models/readiness.json") }
    private var voiceReceiptURL: URL { baseDirectory.appendingPathComponent("Models/voice-readiness.json") }
    private let voiceRuntimeID = "voice-\(VoiceIdentityModel.revision.prefix(8))-active-centered-v2"
    var voiceModelDirectory: URL { baseDirectory.appendingPathComponent("Models/VoiceIdentity", isDirectory: true) }
    var voiceDownloadBytes: Int64 { ModelCatalog.assets.filter { $0.group == .voiceIdentity }.reduce(0) { $0 + $1.bytes } }

    init(baseDirectory: URL) {
        self.baseDirectory = baseDirectory
        refresh()
    }

    func refresh() {
        readyGroups = Set(ModelGroup.allCases.filter { group in
            ModelCatalog.assets.filter { $0.group == group }.allSatisfy { asset in
                stamp(asset)?.bytes == Int(asset.bytes)
            }
        })
        isReadyForUse = false
        if let data = try? Data(contentsOf: receiptURL), let receipt = try? JSONDecoder().decode(ModelReadinessReceipt.self, from: data),
           receipt.runtime == runtimeID, readyGroups.isSuperset(of: [.moss, .nemotron3]) {
            isReadyForUse = defaultAssets.allSatisfy { receipt.files[$0.sha256] == stamp($0) }
        }
        isVoiceModelReady = false
        if let data = try? Data(contentsOf: voiceReceiptURL),
           let receipt = try? JSONDecoder().decode(ModelReadinessReceipt.self, from: data),
           receipt.runtime == voiceRuntimeID, readyGroups.contains(.voiceIdentity) {
            isVoiceModelReady = voiceAssets.allSatisfy { receipt.files[$0.sha256] == stamp($0) }
        }
    }

    var hasPartialDownloads: Bool {
        ModelCatalog.assets.contains { FileManager.default.fileExists(atPath: partialURL($0).path) }
    }
    private var defaultAssets: [ModelAsset] { ModelCatalog.assets.filter { [.moss, .nemotron3].contains($0.group) } }
    private var voiceAssets: [ModelAsset] { ModelCatalog.assets.filter { $0.group == .voiceIdentity } }

    func install(groups: Set<ModelGroup> = [.moss, .nemotron3]) {
        guard !isInstalling else { return }
        isInstalling = true
        fraction = 0
        downloadedBytes = 0
        let assets = ModelCatalog.assets.filter { groups.contains($0.group) }
        totalBytes = assets.reduce(0) { $0 + $1.bytes }
        task = Task {
            defer { isInstalling = false; task = nil; refresh() }
            do {
                try checkSpace(for: assets)
                var completed: Int64 = 0
                for asset in assets {
                    try Task.checkCancellation()
                    message = "\(asset.group.label) 파일 확인 중"
                    let destination = asset.destination(in: baseDirectory)
                    if !(try await Self.matches(destination, asset: asset)) {
                        try checkSpace(for: [asset], skipExisting: false)
                        let partial = partialURL(asset)
                        var downloaded: URL
                        if try await Self.matches(partial, asset: asset) { downloaded = partial }
                        else {
                            if let size = try? partial.resourceValues(forKeys: [.fileSizeKey]).fileSize, size >= asset.bytes {
                                try FileManager.default.removeItem(at: partial)
                            }
                            message = "\(asset.group.label) 받는 중"
                            let completedBefore = completed
                            let throttle = ProgressThrottle()
                            let operation = ModelDownload(asset: asset, directory: downloads) { [weak self] bytes in
                                guard throttle.shouldReport(bytes, of: asset.bytes) else { return }
                                Task { @MainActor in
                                    guard let self else { return }
                                    self.downloadedBytes = completedBefore + bytes
                                    self.fraction = min(1, Double(self.downloadedBytes) / Double(self.totalBytes))
                                }
                            }
                            downloaded = try await operation.run()
                        }
                        message = "\(asset.group.label) 파일 검증 중"
                        guard try await Self.matches(downloaded, asset: asset) else { throw ModelPreparationError.integrity }
                        try Task.checkCancellation()
                        try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
                        if FileManager.default.fileExists(atPath: destination.path) {
                            _ = try FileManager.default.replaceItemAt(destination, withItemAt: downloaded)
                        } else { try FileManager.default.moveItem(at: downloaded, to: destination) }
                    }
                    completed += asset.bytes
                    downloadedBytes = completed
                    fraction = Double(completed) / Double(totalBytes)
                }
                refresh()
                if groups.contains(.voiceIdentity) {
                    message = "목소리 모델 실행 검사 중"
                    try await VoiceModelCheck.run(directory: voiceModelDirectory)
                    try Task.checkCancellation()
                    let files = Dictionary(uniqueKeysWithValues: voiceAssets.compactMap { asset in stamp(asset).map { (asset.sha256, $0) } })
                    guard files.count == voiceAssets.count else { throw ModelPreparationError.integrity }
                    try JSONEncoder().encode(ModelReadinessReceipt(runtime: voiceRuntimeID, files: files))
                        .write(to: voiceReceiptURL, options: .atomic)
                    message = "목소리 등록 준비 완료"
                }
                if !groups.isDisjoint(with: [.moss, .nemotron3]), readyGroups.isSuperset(of: [.moss, .nemotron3]) {
                    message = "전사 / 화자 분리 실행 검사 중"
                    try await ModelSmokeCheck.run(appBundle: Bundle.main.bundleURL, applicationSupport: baseDirectory)
                    try Task.checkCancellation()
                    let files = Dictionary(uniqueKeysWithValues: defaultAssets.compactMap { asset in stamp(asset).map { (asset.sha256, $0) } })
                    guard files.count == defaultAssets.count else { throw ModelPreparationError.integrity }
                    let receipt = ModelReadinessReceipt(runtime: runtimeID, files: files)
                    try JSONEncoder().encode(receipt).write(to: receiptURL, options: .atomic)
                    message = "사용 준비 완료"
                } else if !groups.contains(.voiceIdentity) { message = "선택한 모델 준비가 끝났습니다." }
            } catch is CancellationError { message = "준비를 중단했습니다. 이어받기를 누르면 받은 부분부터 확인해 계속합니다." }
            catch { message = error.localizedDescription }
        }
    }

    func cancel() { task?.cancel() }
    func cancelAndWait() async { task?.cancel(); await task?.value }

    private func checkSpace(for assets: [ModelAsset], skipExisting: Bool = true) throws {
        try FileManager.default.createDirectory(at: baseDirectory, withIntermediateDirectories: true)
        let values = try baseDirectory.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey])
        guard let available = values.volumeAvailableCapacityForImportantUsage ?? values.volumeAvailableCapacity.map(Int64.init) else { return }
        let needed = assets.reduce(Int64(256 * 1024 * 1024)) { result, asset in
            if skipExisting && stamp(asset)?.bytes == Int(asset.bytes) { return result }
            let partial = (try? partialURL(asset).resourceValues(forKeys: [.fileSizeKey]).fileSize).map(Int64.init) ?? 0
            return result + max(0, asset.bytes - partial)
        }
        guard available >= needed else { throw ModelPreparationError.space(needed) }
    }

    private func partialURL(_ asset: ModelAsset) -> URL { downloads.appendingPathComponent(asset.sha256 + ".partial") }
    private func stamp(_ asset: ModelAsset) -> ModelFileStamp? {
        guard let values = try? asset.destination(in: baseDirectory).resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey]),
              let size = values.fileSize, let date = values.contentModificationDate else { return nil }
        return .init(bytes: size, modifiedMilliseconds: Int64(date.timeIntervalSince1970 * 1000))
    }

    nonisolated static func matches(_ url: URL, asset: ModelAsset) async throws -> Bool {
        try await Task.detached(priority: .utility) {
            // URL resource values can retain the file size from before a resumed download.
            guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
                  let size = attributes[.size] as? NSNumber, size.int64Value == asset.bytes else { return false }
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            var hash = SHA256()
            while let data = try handle.read(upToCount: 1024 * 1024), !data.isEmpty { hash.update(data: data) }
            return hash.finalize().map { String(format: "%02x", $0) }.joined() == asset.sha256
        }.value
    }
}

enum ModelPreparationError: Error, LocalizedError {
    case download, integrity, space(Int64)
    var errorDescription: String? {
        switch self {
        case .download: "모델을 받지 못했습니다. 인터넷 연결을 확인하고 이어받기를 다시 시도해 주세요."
        case .integrity: "받은 모델 파일의 검증에 실패했습니다. 다시 시도해 주세요."
        case .space(let needed): "모델 준비에 필요한 저장 공간이 부족합니다. 최소 \(ByteCountFormatter.string(fromByteCount: needed, countStyle: .file))를 확보해 주세요."
        }
    }
}
