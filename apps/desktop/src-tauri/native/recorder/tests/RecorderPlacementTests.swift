import AppKit

@main struct RecorderPlacementTests {
    static func main() throws {
        let laptop = RecorderDisplay(id: 1, frame: NSRect(x: 0, y: 0, width: 1512, height: 982),
                                     workArea: NSRect(x: 0, y: 30, width: 1512, height: 920),
                                     notch: NSRect(x: 666, y: 950, width: 180, height: 32))
        let external = RecorderDisplay(id: 2, frame: NSRect(x: -1920, y: 200, width: 1920, height: 1080),
                                      workArea: NSRect(x: -1920, y: 230, width: 1920, height: 1020))
        assert(laptop.docked(expanded: false).maxY == laptop.frame.maxY)
        assert(laptop.docked(expanded: true).maxY == laptop.frame.maxY)
        assert(laptop.docked(expanded: false).midX == laptop.notch!.midX)
        assert(laptop.docked(expanded: true).height > laptop.notch!.height)
        let inset = RecorderMetrics.inset
        for (seconds, failure) in [(UInt64(95), false), (5025, false), (10800, false), (359999, false), (0, true)] {
            let text = RecorderMetrics.textWidth(seconds: seconds, failure: failure)
            let shown = failure ? "오류 확인" : RecorderMetrics.clock(seconds)
            assert((shown as NSString).size(withAttributes: [.font: RecorderMetrics.font]).width <= text)
            for expanded in [false, true] {
                let panel = laptop.docked(expanded: expanded, text: text)
                assert(panel.midX == laptop.notch!.midX)
                let mark = laptop.markFrame(floating: false), time = laptop.timeFrame(text: text, floating: false)
                let pause = laptop.primaryControlFrame(index: 0, in: panel), stop = laptop.primaryControlFrame(index: 1, in: panel)
                // Equal outer margins, and both groups clear the notch.
                assert(mark.minX == inset && panel.width - stop.maxX == inset)
                assert(panel.minX + time.maxX <= laptop.notch!.minX - RecorderMetrics.notchGap)
                assert(panel.minX + pause.minX >= laptop.notch!.maxX + RecorderMetrics.notchGap)
                assert(!pause.intersects(stop))
                if expanded {
                    let open = laptop.detailControlFrame(index: 0, count: 2, in: panel), hide = laptop.detailControlFrame(index: 1, count: 2, in: panel)
                    assert(hide.maxX == stop.maxX && open.minX == pause.minX)
                    assert(laptop.titleFrame(controls: 2, in: panel).minX == inset && laptop.titleFrame(controls: 2, in: panel).maxX < open.minX)
                    assert(open.minY >= laptop.headerHeight)
                }
            }
            let floating = RecorderMetrics.floatingWidth(text)
            let first = RecorderDisplay.floatingControlFrame(index: 0, width: floating), last = RecorderDisplay.floatingControlFrame(index: 4, width: floating)
            assert(floating - last.maxX == inset)
            assert(laptop.timeFrame(text: text, floating: true).maxX + RecorderMetrics.groupGap <= first.minX)
            assert(RecorderDisplay.floatingControlFrame(index: 2, width: floating).minX - RecorderDisplay.floatingControlFrame(index: 1, width: floating).maxX == RecorderMetrics.groupGap)
        }
        let short = laptop.docked(expanded: false, text: RecorderMetrics.clockWidth(5)).width
        assert(short < laptop.docked(expanded: false, text: RecorderMetrics.clockWidth(3600)).width)
        let plain = external.docked(expanded: false)
        assert(external.timeFrame(text: RecorderMetrics.clockWidth(0), floating: false).maxX + RecorderMetrics.groupGap <= external.primaryControlFrame(index: 0, in: plain).minX)
        assert(plain.width - external.primaryControlFrame(index: 1, in: plain).maxX == inset)
        assert(external.docked(expanded: false).maxY == external.workArea.maxY)
        assert(external.docked(expanded: true).midX == external.workArea.midX)
        assert(laptop.acceptsDock(at: NSPoint(x: 756, y: 955)))
        assert(!laptop.acceptsDock(at: NSPoint(x: 120, y: 955)))
        assert(!laptop.acceptsDock(at: NSPoint(x: 756, y: 450)))
        assert(external.acceptsDock(at: NSPoint(x: -960, y: 1245)))
        var placement = RecorderPlacement(floating: true, displayID: 2, x: -1800, y: 500)
        assert(placement.resolve(displays: [laptop, external], preferred: 1) == external)
        assert(placement.x == -1800 && placement.y == 500)
        assert(placement.resolve(displays: [laptop], preferred: 1) == laptop)
        assert(laptop.workArea.contains(NSRect(x: placement.x, y: placement.y, width: RecorderMetrics.floatingWidth(RecorderMetrics.clockWidth(0)), height: RecorderMetrics.floatingHeight)))
        let preserved = try JSONDecoder().decode(RecorderPlacement.self, from: JSONEncoder().encode(placement))
        assert(preserved == placement)
        var docked = RecorderPlacement()
        assert(docked.resolve(displays: [laptop, external], preferred: 2) == external)
        assert(docked.resolve(displays: [], preferred: 1) == nil)
        print("Recorder placement: equal margins, clock-sized wings, aligned detail row, notch, no-notch, negative display, detach, redock, monitor removal, persistence passed")
    }
}
