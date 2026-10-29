import Foundation

@main
struct AudioPrepare {
    static func main() async {
        do {
            guard CommandLine.arguments.count == 3 else { throw AudioPreparationError.invalidOutput("입력과 새 출력 경로가 필요합니다.") }
            let audio = try await AnalysisAudioPreparer.prepare(source: URL(fileURLWithPath: CommandLine.arguments[1]), destination: URL(fileURLWithPath: CommandLine.arguments[2]))
            FileHandle.standardOutput.write(try JSONEncoder().encode(audio))
        } catch {
            FileHandle.standardError.write(Data("음성 파일을 준비하지 못했습니다: \(error.localizedDescription)\n".utf8))
            exit(1)
        }
    }
}
