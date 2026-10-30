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
        for expanded in [false, true] {
            let panel = laptop.docked(expanded: expanded)
            let time = laptop.timeFrame(in: panel, floating: false)
            assert(panel.minX + time.maxX <= laptop.notch!.minX - 6)
            let pause = laptop.primaryControlFrame(index: 0, in: panel)
            let stop = laptop.primaryControlFrame(index: 1, in: panel)
            assert(panel.minX + pause.minX >= laptop.notch!.maxX + 8)
            assert(stop.maxX <= panel.width - 8)
            assert(!pause.intersects(stop))
            for value in ["01:35", "1:23:45", "3:00:00", "99:59:59"] {
                assert((value as NSString).size(withAttributes: [.font: NSFont.monospacedDigitSystemFont(ofSize: RecorderTokens.caption, weight: .semibold)]).width <= time.width)
            }
        }
        let plain = external.docked(expanded: false)
        assert(external.timeFrame(in: plain, floating: false).maxX + 8 <= external.primaryControlFrame(index: 0, in: plain).minX)
        assert(external.primaryControlFrame(index: 1, in: plain).maxX <= plain.width - 8)
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
        assert(laptop.workArea.contains(NSRect(x: placement.x, y: placement.y, width: 276, height: 44)))
        let preserved = try JSONDecoder().decode(RecorderPlacement.self, from: JSONEncoder().encode(placement))
        assert(preserved == placement)
        var docked = RecorderPlacement()
        assert(docked.resolve(displays: [laptop, external], preferred: 2) == external)
        assert(docked.resolve(displays: [], preferred: 1) == nil)
        print("Recorder placement: notch, no-notch, negative display, detach, redock, monitor removal, persistence passed")
    }
}
