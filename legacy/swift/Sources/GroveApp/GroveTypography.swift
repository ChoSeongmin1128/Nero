import CoreText
import SwiftUI

enum GroveTypography {
    static let title = Font.custom("Pretendard-SemiBold", size: 22)
    static let heading = Font.custom("Pretendard-SemiBold", size: 17)
    static let body = Font.custom("Pretendard-Regular", size: 15)
    static let bodySmall = Font.custom("Pretendard-Regular", size: 13)
    static let label = Font.custom("Pretendard-Medium", size: 13)

    static func registerFonts() {
        let resourceBundle: Bundle
        if let url = Bundle.main.url(forResource: "Grove_GroveApp", withExtension: "bundle"),
           let packaged = Bundle(url: url) {
            resourceBundle = packaged
        } else {
            resourceBundle = .module
        }
        for name in ["Pretendard-Regular", "Pretendard-Medium", "Pretendard-SemiBold"] {
            guard let url = resourceBundle.url(forResource: name, withExtension: "otf", subdirectory: "Fonts") else {
                NSLog("Grove: bundled font missing: %@", name)
                continue
            }
            var error: Unmanaged<CFError>?
            if !CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) {
                let description = error?.takeRetainedValue().localizedDescription ?? "unknown font registration error"
                NSLog("Grove: font registration: %@", description)
            }
        }
    }
}
