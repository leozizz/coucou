import Foundation

@main
enum FlowKeyLogicTests {

    static var failures = 0

    static func check(_ label: String, _ got: Bool) {
        if got { print("  ✓ \(label)") }
        else   { print("  ✗ \(label)"); failures += 1 }
    }

    static func main() {

        print("\nHold to talk")
        do {
            var k = FlowKeyLogic()
            check("press starts listening at once", k.keyDown(at: 10) == .start)
            check("key repeat does nothing", k.keyDown(at: 10.1) == .none)
            check("release after a hold pastes", k.keyUp(at: 11.2) == .finish)
            check("back to idle", k.phase == .idle)
        }

        print("\nA short tap waits for a second one")
        do {
            var k = FlowKeyLogic()
            _ = k.keyDown(at: 10)
            check("short release does not paste yet", k.keyUp(at: 10.1) == .none)
            check("waiting for the second tap", k.phase == .tapWait(until: 10.1 + FlowKeyLogic.doubleTapWindow))
            check("no tick before the window ends", k.tick(at: 10.3) == .none)
            check("the window ends: dropped", k.tick(at: 10.6) == .cancel)
            check("idle after the drop", k.phase == .idle)
        }

        print("\nDouble tap: hands-free")
        do {
            var k = FlowKeyLogic()
            _ = k.keyDown(at: 10)
            _ = k.keyUp(at: 10.1)
            check("second tap locks", k.keyDown(at: 10.3) == .lock)
            check("release while locked does nothing", k.keyUp(at: 10.4) == .none)
            check("another key while locked does nothing", k.otherKey() == .none)
            check("next tap pastes", k.keyDown(at: 15) == .finish)
            check("idle after", k.phase == .idle)
            check("its release does nothing", k.keyUp(at: 15.1) == .none)
        }

        print("\nA second press after the window, before the tick")
        do {
            var k = FlowKeyLogic()
            _ = k.keyDown(at: 10)
            _ = k.keyUp(at: 10.1)
            check("starts afresh", k.keyDown(at: 11) == .restart)
            check("holding again", k.phase == .holding(since: 11))
        }

        print("\nShortcuts and Esc cancel")
        do {
            var k = FlowKeyLogic()
            _ = k.keyDown(at: 10)
            check("another key while held cancels (a shortcut such as ⌥E)", k.otherKey() == .cancel)
            check("idle", k.phase == .idle)
            check("Esc when idle does nothing", k.escape() == .none)
            _ = k.keyDown(at: 20)
            check("Esc while held cancels", k.escape() == .cancel)
            _ = k.keyDown(at: 30); _ = k.keyUp(at: 30.1); _ = k.keyDown(at: 30.2)
            check("Esc while hands-free cancels", k.escape() == .cancel)
            _ = k.keyDown(at: 40); _ = k.keyUp(at: 40.1)
            check("another key between the two taps cancels", k.otherKey() == .cancel)
            check("release when idle does nothing", k.keyUp(at: 41) == .none)
        }

        print("\nFrequency bands")
        do {
            let e = FlowBands.edges()
            check("ten edges for nine bands", e.count == FlowBands.count + 1)
            check("from the graves", abs(e.first! - FlowBands.lowHz) < 0.001)
            check("to the aigus", abs(e.last! - FlowBands.highHz) < 0.001)
            check("rising", zip(e, e.dropFirst()).allSatisfy { $0 < $1 })
            for rate in [44_100.0, 48_000.0, 16_000.0] {
                let r = FlowBands.binRanges(fftSize: 1024, sampleRate: rate)
                check("nine bands at \(Int(rate)) Hz", r.count == FlowBands.count)
                check("each band has a bin at \(Int(rate)) Hz", r.allSatisfy { !$0.isEmpty })
                check("bands do not overlap at \(Int(rate)) Hz", zip(r, r.dropFirst()).allSatisfy { $0.upperBound <= $1.lowerBound })
                check("bands stay under Nyquist at \(Int(rate)) Hz", r.allSatisfy { $0.upperBound <= 512 })
            }
            check("silence is low", FlowBands.level(db: -90) == 0)
            check("a loud voice is full", FlowBands.level(db: -20) == 1)
            check("in between", FlowBands.level(db: -54) > 0.3 && FlowBands.level(db: -54) < 0.7)
        }

        if failures == 0 {
            print("\nAll tests passed.")
            exit(0)
        } else {
            print("\n\(failures) test(s) failed.")
            exit(1)
        }
    }
}
