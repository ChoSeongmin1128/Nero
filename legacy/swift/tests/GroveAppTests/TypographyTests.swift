import AppKit
import Testing
@testable import GroveApp

struct TypographyTests {
    @Test @MainActor func bundledPretendardRegistersWithExpectedNames() {
        GroveTypography.registerFonts()
        for name in ["Pretendard-Regular", "Pretendard-Medium", "Pretendard-SemiBold"] {
            let font = NSFont(name: name, size: 15)
            #expect(font != nil)
            #expect(font?.familyName == "Pretendard")
        }
    }
}
