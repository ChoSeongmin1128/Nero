import Foundation

enum AudioPreparationError: Error, LocalizedError {
    case invalidOutput(String)

    var errorDescription: String? {
        switch self {
        case .invalidOutput(let detail): "처리 결과를 확인할 수 없습니다. \(detail)"
        }
    }
}
