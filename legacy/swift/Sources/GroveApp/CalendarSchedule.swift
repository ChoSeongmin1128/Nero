import AppKit
@preconcurrency import EventKit
import SwiftUI

struct ScheduledMeeting: Codable, Hashable, Identifiable, Sendable {
    let id: String
    let calendarID: String
    let title: String
    let start: Date
    let end: Date
    let calendarName: String

    func isReminderDue(at now: Date, leadMinutes: Int) -> Bool {
        start <= now.addingTimeInterval(Double(leadMinutes * 60)) && end > now
            && start >= now.addingTimeInterval(-600)
    }
}

struct ScheduleCalendar: Identifiable {
    let id: String
    let title: String
}

@MainActor
final class CalendarSchedule: ObservableObject {
    @Published private(set) var enabled: Bool
    @Published private(set) var calendars: [ScheduleCalendar] = []
    @Published private(set) var meetings: [ScheduledMeeting] = []
    @Published private(set) var message: String?
    @Published var selectedCalendarIDs: Set<String>
    @Published var leadMinutes: Int { didSet { defaults.set(leadMinutes, forKey: "calendarLeadMinutes") } }
    var onRecord: ((ScheduledMeeting) -> Void)?
    var canRecord: (() -> Bool)?
    private let eventStore: EKEventStore
    private let defaults: UserDefaults
    private var timer: Timer?
    private var observer: NSObjectProtocol?
    private var dismissed: Set<String> = []
    private var snoozed: [String: Date] = [:]
    private var panel: NSPanel?
    private var shownID: String?
    private var enableRequestID = UUID()
    private let requestPermission: @MainActor () async throws -> Bool

    init(defaults: UserDefaults = .standard, requestPermission: (@MainActor () async throws -> Bool)? = nil) {
        self.defaults = defaults
        let eventStore = EKEventStore()
        self.eventStore = eventStore
        self.requestPermission = requestPermission ?? { try await eventStore.requestFullAccessToEvents() }
        enabled = defaults.bool(forKey: "calendarEnabled")
        selectedCalendarIDs = Set(defaults.stringArray(forKey: "calendarIDs") ?? [])
        leadMinutes = defaults.object(forKey: "calendarLeadMinutes") == nil ? 5 : defaults.integer(forKey: "calendarLeadMinutes")
        observer = NotificationCenter.default.addObserver(forName: .EKEventStoreChanged, object: eventStore, queue: .main) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func start() {
        guard timer == nil else { return }
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refresh() }
        }
    }

    func setEnabled(_ value: Bool) async {
        let requestID = UUID()
        enableRequestID = requestID
        if value {
            do {
                let allowed = try await requestPermission()
                guard enableRequestID == requestID, !Task.isCancelled else { return }
                guard allowed else {
                    message = "캘린더 접근이 허용되지 않았습니다. 시스템 설정에서 Grove의 캘린더 접근을 허용해 주세요."
                    return
                }
            } catch {
                guard enableRequestID == requestID, !Task.isCancelled else { return }
                message = error.localizedDescription
                return
            }
        }
        enabled = value
        defaults.set(value, forKey: "calendarEnabled")
        message = nil
        refresh()
    }

    func saveSelection() {
        defaults.set(Array(selectedCalendarIDs).sorted(), forKey: "calendarIDs")
        refresh()
    }

    func refresh() {
        guard enabled else { meetings = []; hidePanel(); return }
        guard EKEventStore.authorizationStatus(for: .event) == .fullAccess else {
            meetings = []; hidePanel(); message = "캘린더 접근을 허용해 주세요."; return
        }
        let sources = eventStore.calendars(for: .event)
        calendars = sources.map { .init(id: $0.calendarIdentifier, title: "\($0.title) (\($0.source.title))") }
        if defaults.object(forKey: "calendarIDs") == nil {
            selectedCalendarIDs = Set(sources.map(\.calendarIdentifier))
            defaults.set(Array(selectedCalendarIDs), forKey: "calendarIDs")
        }
        let selected = sources.filter { selectedCalendarIDs.contains($0.calendarIdentifier) }
        guard !selected.isEmpty else { meetings = []; hidePanel(); return }
        let now = Date()
        let predicate = eventStore.predicateForEvents(withStart: now.addingTimeInterval(-600),
            end: Calendar.current.startOfDay(for: now).addingTimeInterval(2 * 86400), calendars: selected)
        let events = eventStore.events(matching: predicate).filter { event in
            !event.isAllDay && event.status != .canceled && event.endDate > now
                && !(event.attendees?.contains { $0.isCurrentUser && $0.participantStatus == .declined } ?? false)
        }
        var seen: Set<String> = []
        meetings = events.compactMap { event in
            let id = "\(event.calendar.calendarIdentifier):\(event.calendarItemIdentifier):\(event.startDate.timeIntervalSince1970)"
            guard seen.insert(id).inserted else { return nil }
            return ScheduledMeeting(id: id, calendarID: event.calendar.calendarIdentifier,
                title: event.title?.isEmpty == false ? event.title : "제목 없는 일정",
                start: event.startDate, end: event.endDate, calendarName: event.calendar.title)
        }.sorted { $0.start < $1.start }
        message = nil
        let valid = Set(meetings.map(\.id))
        dismissed.formIntersection(valid)
        snoozed = snoozed.filter { valid.contains($0.key) }
        let next = meetings.first {
            $0.isReminderDue(at: now, leadMinutes: leadMinutes) && !dismissed.contains($0.id)
                && (snoozed[$0.id].map { $0 <= now } ?? true)
        }
        if let next, canRecord?() != false { show(next) } else { hidePanel() }
    }

    func record(_ meeting: ScheduledMeeting) {
        guard canRecord?() != false else { return }
        dismissed.insert(meeting.id)
        hidePanel()
        NSApplication.shared.activate(ignoringOtherApps: true)
        onRecord?(meeting)
    }

    func dismiss(_ meeting: ScheduledMeeting, snooze: Bool = false) {
        if snooze { snoozed[meeting.id] = Date().addingTimeInterval(300) }
        else { dismissed.insert(meeting.id) }
        hidePanel()
    }

    func previewReminder() {
        let now = Date()
        show(.init(id: "preview", calendarID: "preview", title: "일정 알림 미리보기", start: now.addingTimeInterval(300),
                   end: now.addingTimeInterval(3600), calendarName: "미리보기"), preview: true)
    }

    private func show(_ meeting: ScheduledMeeting, preview: Bool = false) {
        guard shownID != meeting.id else { return }
        hidePanel()
        let width = min(440, (NSScreen.main?.visibleFrame.width ?? 980) - 40)
        let panel = NSPanel(contentRect: NSRect(x: 0, y: 0, width: width, height: 40),
            styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        panel.isReleasedWhenClosed = false
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.contentView = NSHostingView(rootView: MeetingReminderBanner(meeting: meeting,
            record: { [weak self] in if preview { self?.hidePanel() } else { self?.record(meeting) } },
            snooze: { [weak self] in self?.dismiss(meeting, snooze: true) },
            close: { [weak self] in self?.dismiss(meeting) }))
        if let screen = NSScreen.main {
            panel.setFrameOrigin(NSPoint(x: screen.visibleFrame.midX - width / 2, y: screen.visibleFrame.maxY - 48))
        }
        shownID = meeting.id
        self.panel = panel
        panel.orderFrontRegardless()
    }

    private func hidePanel() {
        panel?.orderOut(nil)
        panel = nil
        shownID = nil
    }
}

struct MeetingReminderBanner: View {
    let meeting: ScheduledMeeting
    let record: () -> Void
    let snooze: () -> Void
    let close: () -> Void
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "waveform").font(.system(size: 14)).foregroundStyle(.white)
            Text(meeting.title).font(.system(size: 13, weight: .semibold)).lineLimit(1).foregroundStyle(.white)
                .help(meeting.title).layoutPriority(-1)
            TimelineView(.periodic(from: .now, by: 30)) { context in
                let minutes = max(0, Int(ceil(meeting.start.timeIntervalSince(context.date) / 60)))
                Text(minutes > 0 ? "\(minutes)분 후" : "지금")
                    .font(.system(size: 12)).foregroundStyle(.white.opacity(0.65)).fixedSize()
            }
            Spacer(minLength: 4)
            Button(action: record) {
                Text("녹음 시작").font(.system(size: 12, weight: .medium)).foregroundStyle(.black)
                    .padding(.horizontal, 10).padding(.vertical, 6)
                    .background(.white, in: RoundedRectangle(cornerRadius: 7))
            }.buttonStyle(.plain).fixedSize(horizontal: true, vertical: false)
            Menu { Button("5분 뒤 알림", action: snooze); Button("이 일정 알림 닫기", action: close) } label: {
                Image(systemName: "chevron.down")
            }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().foregroundStyle(.white)
                .accessibilityLabel("일정 알림 옵션")
            Button(action: close) { Image(systemName: "xmark").frame(width: 20, height: 24) }
                .buttonStyle(.plain).foregroundStyle(.white)
                .accessibilityLabel("일정 알림 닫기")
        }.padding(.horizontal, 12).frame(height: 40)
            .background(Color(nsColor: NSColor(calibratedWhite: 0.09, alpha: 0.98)), in: Capsule())
    }
}
