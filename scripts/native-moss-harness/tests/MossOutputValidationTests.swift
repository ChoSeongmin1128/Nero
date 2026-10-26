import Foundation

@main
struct MossOutputValidationTests {
    static func main() {
        check(MossOutputValidation.hasConfiguredEOS(terminationReason: "eos", terminalTokenId: 151645, configuredEosTokenIds: [151645], tokenizerEosTokenId: 151645), "only configured model and tokenizer EOS completes generation")
        for (reason, token, configured, tokenizer) in [
            ("eos", 151643, [151645], 151645),
            ("eos", 151643, [151643, 151645], 151645),
            ("eos", 151645, [151643, 151645], 151645),
            ("eos", 151645, [151645], 151643),
            ("length", 151645, [151645], 151645),
            ("repetition", 151645, [151645], 151645),
        ] {
            check(!MossOutputValidation.hasConfiguredEOS(terminationReason: reason, terminalTokenId: token, configuredEosTokenIds: configured, tokenizerEosTokenId: tokenizer), "PAD, mismatched configuration and other stops cannot complete generation")
        }
        check(!MossOutputValidation.hasConfiguredEOS(terminationReason: "eos", terminalTokenId: nil, configuredEosTokenIds: [151645], tokenizerEosTokenId: 151645), "missing terminal token is incomplete")
        let first = segment(0, 1, "[S1] 첫 발언", "S1")
        let second = segment(2, 3, "[S2] 다음 발언", "S2")
        let raw = "[0][S1]첫 발언[1]\n[2][S2]다음 발언[3]"
        check(MossOutputValidation.isComplete(raw, segments: [first, second]), "complete multi-speaker output")
        check(MossOutputValidation.isComplete("[0,25][S1]  문장\n [1,5]", segments: [segment(0.25, 1.5, "[S1] 문장", "S1")]), "comma timestamps and body whitespace")
        check(MossOutputValidation.isComplete("[0][S1]겹침[2][1][S2]동시 발언[3]", segments: [segment(0, 2, "[S1] 겹침", "S1"), segment(1, 3, "[S2] 동시 발언", "S2")]), "overlap is allowed")

        for invalid in [
            "[0][S1]첫 발언[1][9][S2]버려질 발언[8]",
            "[9][S2]버려질 발언[8][0][S1]첫 발언[1]",
            "[0][S1]첫 발언[1][9][S2]   [8]",
            "[0][S1]첫 발언[1][" + String(repeating: "9", count: 400) + "][S2]숫자 초과[10]",
            "[0][S1]첫 발언[1]끝나지 않은 발언",
            "[0][S1]첫 발언[1][2][S2]",
        ] {
            check(!MossOutputValidation.isComplete(invalid, segments: [first]), "invalid raw cannot hide a dropped segment")
        }
        check(!MossOutputValidation.isComplete("[9][S2]버려질 발언[8]", segments: [segment(0, 10, "[9][S2]버려질 발언[8]", nil)]), "fallback cannot disguise an invalid timestamp")
        for changed in [
            [first],
            [first, second, second],
            [second, first],
            [first, segment(2.1, 3, "[S2] 다음 발언", "S2")],
            [first, segment(2, 3.1, "[S2] 다음 발언", "S2")],
            [first, segment(2, 3, "[S2] 변경된 발언", "S2")],
            [first, segment(2, 3, "[S2] 다음 발언", "S3")],
            [first, segment(2, 3, "[S2] 다음 발언", nil)],
            [first, segment(.infinity, 3, "[S2] 다음 발언", "S2")],
            [first, segment(2, .nan, "[S2] 다음 발언", "S2")],
        ] {
            check(!MossOutputValidation.isComplete(raw, segments: changed), "returned segments must correspond one to one")
        }

        check(MossOutputValidation.isComplete("", segments: []), "empty silence")
        check(MossOutputValidation.isComplete(" \n\t", segments: [segment(0, 4, " \n", nil)]), "whitespace silence fallback")
        check(!MossOutputValidation.isComplete("", segments: [first]), "silence cannot invent speech")
        check(!MossOutputValidation.isComplete(raw, segments: []), "speech cannot disappear")
        check(MossOutputValidation.isComplete("[0][S1] \n [1][2][S2]다음 발언[3]", segments: [second]), "empty tagged body can be omitted beside speech")
        check(!MossOutputValidation.isComplete("[0][S1] [1]", segments: [segment(0, 4, "[0][S1] [1]", nil)]), "tag-only fallback is not speech")

        let overflowRaw = "[0][S1]첫 발언[1][2][S2]숫자 초과[" + String(repeating: "9", count: 400) + "]"
        let overflowSegments = [first, segment(2, .infinity, "[S2] 숫자 초과", "S2")]
        let validated = MossOutputValidation.validatedSegments(overflowRaw, segments: overflowSegments)
        check(validated == nil, "nonfinite parser output has no validated segments")
        let retained: [String: Any] = [
            "rawText": overflowRaw,
            "hasUnparsedText": validated == nil,
            "segments": validated ?? [],
        ]
        guard let encoded = try? JSONSerialization.data(withJSONObject: retained),
              let saved = (try? JSONSerialization.jsonObject(with: encoded)) as? [String: Any] else {
            preconditionFailure("incomplete output must retain serializable raw evidence")
        }
        check(saved["rawText"] as? String == overflowRaw, "overflow raw text is preserved")
        check(saved["hasUnparsedText"] as? Bool == true, "overflow output is incomplete")
        check((saved["segments"] as? [[String: Any]])?.isEmpty == true, "incomplete output cannot publish partial parsed segments")
        check(MossOutputValidation.validatedSegments(raw, segments: [first, second])?.count == 2, "complete output keeps validated segments")
        print("MOSS output validation checks passed")
    }

    private static func segment(_ start: Double, _ end: Double, _ text: String, _ speaker: String?) -> [String: Any] {
        var value: [String: Any] = ["start": start, "end": end, "text": text]
        if let speaker { value["speaker_id"] = speaker }
        return value
    }

    private static func check(_ condition: Bool, _ name: String) {
        precondition(condition, name)
    }
}
