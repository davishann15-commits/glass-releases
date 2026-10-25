import SwiftUI
import UIKit

/// Touch gestures beyond taps and the system's swipe-back (which works from anywhere
/// on a page): swipes on the player bar and Now Playing's artwork, pulling Now
/// Playing down to close it, and swiping a song to queue it (left on pages, where a
/// rightward swipe goes back; right or left in Now Playing). Each also has a VoiceOver
/// action where it's attached.
enum Haptic {
    static func tick() { UIImpactFeedbackGenerator(style: .light).impactOccurred() }
    static func commit() { UIImpactFeedbackGenerator(style: .medium).impactOccurred() }
}

/// A row's tap that ends a swipe isn't a tap: rows check this before playing.
@MainActor enum SwipeGuard {
    static var until = Date.distantPast
    static var active: Bool { Date() < until }
    static func hold(_ seconds: Double = 0.4) { until = Date().addingTimeInterval(seconds) }
}

/// Which way a drag is going, decided once at its start so a slightly diagonal
/// scroll doesn't also swipe.
private enum DragAxis { case horizontal, vertical }

/// Horizontal swipes: `left` when the finger moves left past `threshold` (or flicks),
/// `right` for the other way. The view follows the finger, damped, and springs back.
/// Vertical drags are left alone (scroll views keep working).
struct HorizontalSwipe: ViewModifier {
    var left: (() -> Void)?
    var right: (() -> Void)?
    var threshold: CGFloat = 70
    var follow: CGFloat = 0.55
    @State private var dx: CGFloat = 0
    @State private var axis: DragAxis?
    @State private var armed = false

    func body(content: Content) -> some View {
        content
            .offset(x: dx)
            .simultaneousGesture(
                DragGesture(minimumDistance: 14)
                    .onChanged { v in
                        if axis == nil { axis = abs(v.translation.width) > abs(v.translation.height) * 1.4 ? .horizontal : .vertical }
                        guard axis == .horizontal else { return }
                        var x = v.translation.width
                        if x < 0, left == nil { x = 0 }
                        if x > 0, right == nil { x = 0 }
                        dx = x * follow
                        let past = abs(x) > threshold
                        if past != armed { armed = past; if past { Haptic.tick() } }
                    }
                    .onEnded { v in
                        defer {
                            axis = nil
                            armed = false
                            withAnimation(.spring(response: 0.32, dampingFraction: 0.82)) { dx = 0 }
                        }
                        guard axis == .horizontal else { return }
                        let x = v.translation.width, flick = v.predictedEndTranslation.width
                        if x < -threshold || flick < -threshold * 2.4, let left {
                            Haptic.commit()
                            left()
                        } else if x > threshold || flick > threshold * 2.4, let right {
                            Haptic.commit()
                            right()
                        }
                    }
            )
    }
}

/// Pulling down: `y` follows the finger; past 120 pt (or a flick) `done` runs.
struct PullDown: ViewModifier {
    @Binding var y: CGFloat
    var simultaneous = false
    let done: () -> Void
    @State private var axis: DragAxis?

    private var drag: some Gesture {
        DragGesture(minimumDistance: 12, coordinateSpace: .global)
            .onChanged { v in
                if axis == nil { axis = v.translation.height > 0 && v.translation.height > abs(v.translation.width) * 1.2 ? .vertical : .horizontal }
                guard axis == .vertical else { return }
                y = max(0, v.translation.height)
            }
            .onEnded { v in
                defer { axis = nil }
                guard axis == .vertical else { return }
                if v.translation.height > 120 || v.predictedEndTranslation.height > 320 {
                    Haptic.commit()
                    done()
                } else {
                    withAnimation(.spring(response: 0.35, dampingFraction: 0.86)) { y = 0 }
                }
            }
    }

    func body(content: Content) -> some View {
        if simultaneous { content.simultaneousGesture(drag) } else { content.gesture(drag) }
    }
}

/// Which way a song swipes to join the queue. On pages a rightward swipe goes back,
/// so songs swipe left; in Now Playing (nothing to go back to) they swipe right, as in
/// Spotify, and left as well.
enum QueueSwipe { case left, right }

private struct QueueSwipeKey: EnvironmentKey {
    static let defaultValue: Set<QueueSwipe> = [.left]
}

extension EnvironmentValues {
    var queueSwipe: Set<QueueSwipe> {
        get { self[QueueSwipeKey.self] }
        set { self[QueueSwipeKey.self] = newValue }
    }
}

/// A song swiped joins the queue: the row follows the finger and "＋ queue" shows
/// behind it; past the threshold a tick, on release the song is queued.
struct SwipeToQueue: ViewModifier {
    @Environment(Store.self) private var store
    @Environment(\.queueSwipe) private var ways
    /// The Spotify song to queue (nil, and no `queue`: the row doesn't swipe).
    let uri: String?
    /// Queues something else instead (Local Files songs into Local Files' own queue).
    var queue: (() -> Void)? = nil
    @State private var dx: CGFloat = 0
    @State private var armed = false
    private let threshold: CGFloat = 80

    private func enqueue() {
        if let queue { queue() } else if let uri { store.core.queue(uris: [uri]) }
    }

    func body(content: Content) -> some View {
        if uri != nil || queue != nil {
            let look = Look.shared
            content
                .offset(x: dx)
                .background(alignment: dx > 0 ? .leading : .trailing) {
                    if dx != 0 {
                        Label("queue", systemImage: "text.badge.plus")
                            .gFont(12.5)
                            .foregroundStyle(armed ? look.onAccent : look.accent)
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(Capsule().fill(armed ? look.accent : look.selection))
                            .opacity(min(1, abs(dx) / 50))
                            .padding(.horizontal, 6)
                    }
                }
                // A UIKit pan that starts only for a sideways drag the allowed way: a
                // rightward drag on a page is left to the system's swipe-back (a SwiftUI
                // drag gesture would take it), and vertical ones to the scroll view.
                .gesture(QueuePan(ways: ways) { w, done in
                    if !done {
                        SwipeGuard.hold()
                        let x = abs(w)
                        let shown = x > threshold ? threshold + (x - threshold) * 0.3 : x
                        dx = w < 0 ? -shown : shown
                        let past = x > threshold
                        if past != armed { armed = past; if past { Haptic.tick() } }
                        return
                    }
                    SwipeGuard.hold(0.35)
                    if armed {
                        Haptic.commit()
                        enqueue()
                    }
                    armed = false
                    withAnimation(.spring(response: 0.3, dampingFraction: 0.82)) { dx = 0 }
                })
                .accessibilityAction(named: "Add to queue") { enqueue() }
        } else {
            content
        }
    }
}

/// The pan behind `SwipeToQueue`: `moved(translation, ended)` as the finger goes and
/// when it lets go (or the pan is cancelled). It begins only for a mostly sideways
/// drag in one of `ways`.
struct QueuePan: UIGestureRecognizerRepresentable {
    let ways: Set<QueueSwipe>
    let moved: (CGFloat, Bool) -> Void

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let g = UIPanGestureRecognizer()
        g.delegate = context.coordinator
        return g
    }

    func updateUIGestureRecognizer(_ recognizer: UIPanGestureRecognizer, context: Context) {
        context.coordinator.ways = ways
    }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator(ways: ways) }

    func handleUIGestureRecognizerAction(_ g: UIPanGestureRecognizer, context: Context) {
        let w = g.translation(in: g.view).x
        switch g.state {
        case .changed: moved(w, false)
        case .ended, .cancelled, .failed: moved(w, true)
        default: break
        }
    }

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var ways: Set<QueueSwipe>
        init(ways: Set<QueueSwipe>) { self.ways = ways }

        func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
            guard let pan = g as? UIPanGestureRecognizer else { return false }
            let v = pan.velocity(in: pan.view)
            guard abs(v.x) > abs(v.y) * 1.6 else { return false }
            return v.x < 0 ? ways.contains(.left) : ways.contains(.right)
        }
    }
}

/// A heart that pops up over the artwork when a double-tap likes the song.
struct LikePop: View {
    let shown: Bool
    var body: some View {
        Image(systemName: "heart.fill")
            .font(.system(size: 72, weight: .regular))
            .foregroundStyle(.white)
            .shadow(color: .black.opacity(0.35), radius: 12)
            .scaleEffect(shown ? 1 : 0.4)
            .opacity(shown ? 0.95 : 0)
            .animation(.spring(response: 0.3, dampingFraction: 0.6), value: shown)
            .allowsHitTesting(false)
    }
}
