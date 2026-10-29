import SwiftUI

struct MinutesView: View {
    let meeting: MeetingRecord

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                if meeting.status == .processing || meeting.status == .recording {
                    processingState
                } else if meeting.claims.isEmpty {
                    emptyState
                } else {
                    claimSection(
                        title: "결정",
                        symbol: "checkmark.seal",
                        claims: meeting.claims.filter { $0.kind == .decision }
                    )
                    claimSection(
                        title: "할 일",
                        symbol: "square.and.pencil",
                        claims: meeting.claims.filter { $0.kind == .action }
                    )
                }
            }
            .padding(34)
            .frame(maxWidth: 820, alignment: .leading)
        }
    }

    private var processingState: some View {
        HStack(spacing: 14) {
            ProgressView().controlSize(.small)
            VStack(alignment: .leading, spacing: 4) {
                Text("원본을 보존했습니다")
                    .font(.headline)
                Text("전사와 근거 연결이 끝나면 회의록을 표시합니다.")
                    .foregroundStyle(.secondary)
            }
        }
        .padding(20)
        .background(GroveTheme.surface, in: RoundedRectangle(cornerRadius: 12))
    }

    private var emptyState: some View {
        ContentUnavailableView {
            Label("회의록 항목이 없습니다", systemImage: "doc.text.magnifyingglass")
        } description: {
            Text("Grove는 원문 근거가 없는 결정이나 할 일을 자동으로 만들지 않습니다.")
        }
        .frame(maxWidth: .infinity, minHeight: 360)
    }

    @ViewBuilder
    private func claimSection(title: String, symbol: String, claims: [EvidenceClaim]) -> some View {
        if !claims.isEmpty {
            VStack(alignment: .leading, spacing: 12) {
                Label(title, systemImage: symbol)
                    .font(.title3.weight(.semibold))
                ForEach(claims) { claim in
                    EvidenceClaimRow(claim: claim)
                }
            }
        }
    }
}

private struct EvidenceClaimRow: View {
    let claim: EvidenceClaim

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            VStack(spacing: 0) {
                Circle()
                    .fill(claim.isReviewed ? GroveTheme.grove : GroveTheme.evidence)
                    .frame(width: 9, height: 9)
                Rectangle()
                    .fill(GroveTheme.evidence.opacity(0.25))
                    .frame(width: 1, height: 42)
            }
            VStack(alignment: .leading, spacing: 8) {
                Text(claim.text)
                    .font(.body.weight(.medium))
                HStack(spacing: 10) {
                    if let owner = claim.owner {
                        Label(owner, systemImage: "person")
                    }
                    Label("근거 \(claim.sourceSegmentIDs.count)개", systemImage: "waveform.path")
                    if !claim.isReviewed {
                        Label("확인 필요", systemImage: "exclamationmark.circle")
                            .foregroundStyle(GroveTheme.evidence)
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 8)
    }
}
