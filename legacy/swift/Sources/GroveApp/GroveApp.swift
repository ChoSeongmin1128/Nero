import SwiftUI

@main
struct GroveApp: App {
    @StateObject private var store: GroveStore

    init() {
        GroveTypography.registerFonts()
        #if DEBUG
        let arguments = CommandLine.arguments
        let profile = arguments.firstIndex(of: "--qa-profile").flatMap { index in
            arguments.indices.contains(index + 1) ? URL(fileURLWithPath: arguments[index + 1]) : nil
        }
        _store = StateObject(wrappedValue: GroveStore(baseDirectory: profile))
        #else
        _store = StateObject(wrappedValue: GroveStore())
        #endif
    }

    var body: some Scene {
        Window("Grove", id: "workspace") {
            RootView(store: store)
                .frame(minWidth: 980, minHeight: 640)
                .task { store.calendarSchedule.start(); await store.applePreparation.refresh() }
        }
        .defaultSize(width: 1260, height: 780)
        .windowToolbarStyle(.unifiedCompact(showsTitle: false))
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("새 회의") {
                    store.present(.newMeeting())
                }
                .keyboardShortcut("n")
                .disabled(!store.canPresentNewMeeting)

                Button("파일 가져오기…") {
                    store.requestFileImport()
                }
                .keyboardShortcut("o")
                .disabled(!store.canPresentNewMeeting)
            }
            CommandGroup(replacing: .appTermination) {
                Button("Grove 종료") {
                    Task {
                        if await store.prepareToQuit() { NSApplication.shared.terminate(nil) }
                    }
                }.keyboardShortcut("q")
            }
        }

        Settings {
            SettingsView(store: store)
                .frame(width: 620, height: 600)
        }
    }
}
