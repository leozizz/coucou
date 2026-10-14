#if !APPSTORE
import AppKit
import SwiftUI

/// What the pill shows.
@MainActor
final class FlowPillModel: ObservableObject {
    enum Phase: Equatable {
        case hidden
        /// The key is held: the dots follow the voice.
        case listening
        /// Hands-free (double tap): same, with an orange ring.
        case locked
        /// Released: the Mac turns the voice into words.
        case thinking
        /// No Accessibility permission: the words are on the clipboard.
        case copied
        /// The microphone or speech recognition could not start.
        case failed
    }
    @Published var phase: Phase = .hidden
}

/// Eases the dots toward the microphone's levels, frame by frame (fast up, slower down), and
/// makes them ripple while the Mac turns the voice into words.
final class FlowLevelSmoother {
    private var values = [Double](repeating: 0, count: FlowBands.count)
    private var last: Date?

    func frame(at date: Date, target: [Float]?, thinking: Bool) -> [Double] {
        let dt = min(0.1, last.map { date.timeIntervalSince($0) } ?? 1.0 / 60)
        last = date
        let t = date.timeIntervalSinceReferenceDate
        for i in values.indices {
            var goal: Double
            if thinking {
                // A small wave running through the dots.
                goal = 0.10 + 0.22 * max(0, sin(t * 7.5 - Double(i) * 0.75))
            } else {
                let level = Double(target?[safe: i] ?? 0)
                // A touch of life so a steady sound does not freeze the dots.
                goal = level * (0.86 + 0.14 * sin(t * 11 + Double(i) * 1.7))
            }
            let rate = goal > values[i] ? 28.0 : 10.0
            values[i] += (goal - values[i]) * min(1, rate * dt)
        }
        return values
    }

    func reset() {
        values = [Double](repeating: 0, count: FlowBands.count)
        last = nil
    }
}

private extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}

/// The pill at the bottom of the screen, like Wispr Flow's bar: a black capsule, a small Mochi,
/// and nine dots that rise and fall with the graves (left) and the aigus (right) of the voice.
struct FlowPillView: View {
    @ObservedObject var model: FlowPillModel
    let bands: FlowAudioBands
    let smoother: FlowLevelSmoother

    private var visible: Bool { model.phase != .hidden }

    var body: some View {
        VStack {
            Spacer(minLength: 0)
            capsule
                .scaleEffect(visible ? 1 : 0.55, anchor: .bottom)
                .opacity(visible ? 1 : 0)
                .offset(y: visible ? 0 : 6)
                .animation(.spring(response: 0.32, dampingFraction: 0.72), value: model.phase)
        }
        .frame(width: FlowPillController.size.width, height: FlowPillController.size.height)
    }

    private var capsule: some View {
        HStack(spacing: 9) {
            mochi
            switch model.phase {
            case .copied:
                Text("flow.copied")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(Color(hex: "#E8E9EC"))
                    .fixedSize()
            case .failed:
                Text("flow.failed")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(Color(hex: "#E8E9EC"))
                    .fixedSize()
            default:
                dots
            }
        }
        .padding(.horizontal, 13)
        .frame(height: 34)
        .background(
            Capsule()
                .fill(Color.black.opacity(0.94))
                .overlay(Capsule().strokeBorder(Color.white.opacity(0.16), lineWidth: 1))
                .overlay(Capsule().strokeBorder(Color(hex: "#F97316").opacity(model.phase == .locked ? 0.85 : 0), lineWidth: 1.5))
        )
        .shadow(color: .black.opacity(0.35), radius: 10, y: 4)
    }

    /// Mochi's face, tiny: two eyes that squint while he works out the words.
    private var mochi: some View {
        let thinking = model.phase == .thinking
        return RoundedRectangle(cornerRadius: 6.5, style: .continuous)
            .fill(LinearGradient(colors: [Color(hex: "#F5F6F8"), Color(hex: "#C9CBD1")], startPoint: .top, endPoint: .bottom))
            .frame(width: 19, height: 15)
            .overlay(
                HStack(spacing: 4.5) {
                    ForEach(0..<2, id: \.self) { _ in
                        Capsule()
                            .fill(Color(hex: "#1A1412"))
                            .frame(width: 2.6, height: thinking ? 1.4 : 4.2)
                    }
                }
                .offset(y: 0.5)
                .animation(.easeOut(duration: 0.15), value: thinking)
            )
    }

    private var dots: some View {
        let active = model.phase == .listening || model.phase == .locked || model.phase == .thinking
        return TimelineView(.animation(minimumInterval: 1.0 / 60, paused: !active)) { context in
            let levels = smoother.frame(at: context.date,
                                        target: model.phase == .thinking ? nil : bands.levels,
                                        thinking: model.phase == .thinking)
            HStack(alignment: .center, spacing: 3.2) {
                ForEach(0..<FlowBands.count, id: \.self) { i in
                    Capsule()
                        .fill(Color.white.opacity(0.94))
                        .frame(width: 3.4, height: 3.4 + 16 * CGFloat(levels[i]))
                }
            }
            .frame(height: 22)
        }
    }
}

/// The pill's window: borderless, never takes the focus (the words go to the app you type in),
/// lets clicks through, on every Space and over full-screen apps, at the bottom centre of the
/// screen the mouse is on.
@MainActor
final class FlowPillController {
    static let size = NSSize(width: 280, height: 80)

    let model = FlowPillModel()
    private let smoother = FlowLevelSmoother()
    private var panel: NSPanel?
    private var hideTask: Task<Void, Never>?
    private var bands: FlowAudioBands?

    func show(bands: FlowAudioBands) {
        hideTask?.cancel()
        if panel == nil || self.bands !== bands { build(bands) }
        guard let panel else { return }
        place(panel)
        smoother.reset()
        panel.orderFrontRegardless()
        model.phase = .listening
    }

    /// A short message, then the pill goes away.
    func flash(_ phase: FlowPillModel.Phase) {
        guard let panel else { return }
        hideTask?.cancel()
        if !panel.isVisible { place(panel); panel.orderFrontRegardless() }
        model.phase = phase
        hide(after: 1.8)
    }

    func hide(after delay: TimeInterval = 0) {
        hideTask?.cancel()
        hideTask = Task { @MainActor [weak self] in
            if delay > 0 { try? await Task.sleep(for: .seconds(delay)) }
            guard !Task.isCancelled, let self else { return }
            self.model.phase = .hidden
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled, self.model.phase == .hidden else { return }
            self.panel?.orderOut(nil)
        }
    }

    private func build(_ bands: FlowAudioBands) {
        self.bands = bands
        let view = NSHostingView(rootView: FlowPillView(model: model, bands: bands, smoother: smoother))
        view.frame = NSRect(origin: .zero, size: Self.size)
        let p = NSPanel(contentRect: NSRect(origin: .zero, size: Self.size),
                        styleMask: [.borderless, .nonactivatingPanel],
                        backing: .buffered, defer: false)
        p.isOpaque = false
        p.backgroundColor = .clear
        p.hasShadow = false
        p.level = .statusBar
        p.ignoresMouseEvents = true
        p.hidesOnDeactivate = false
        p.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        p.contentView = view
        panel = p
    }

    private func place(_ panel: NSPanel) {
        let mouse = NSEvent.mouseLocation
        let screen = NSScreen.screens.first { NSMouseInRect(mouse, $0.frame, false) } ?? NSScreen.main
        guard let area = screen?.visibleFrame else { return }
        // Just above the Dock (or the bottom edge when the Dock hides).
        panel.setFrameOrigin(NSPoint(x: area.midX - Self.size.width / 2, y: area.minY + 6))
    }
}
#endif
