import SwiftUI

/// The audio visualizer (desktop ui/visualizer.rs): bars, mirrored bars or a wave,
/// in a gradient from the theme's (or album's) colors, from the samples Glass is
/// actually playing on this device — analysed by the core's output tap and polled
/// at display rate only while this is on screen and something plays here. When music
/// plays on another device there are no samples, and it says so instead of faking it.
struct VisualizerView: View {
    @Environment(Store.self) private var store
    @Environment(\.accessibilityReduceMotion) private var reduce
    @State private var bands: [Float] = []

    var body: some View {
        let look = Look.shared
        let v = look.viz
        let here = store.playback?.here ?? false
        let playing = store.isPlaying && here
        Group {
            if !here && store.isPlaying {
                Text("the visualizer follows music played on this device").gFont(11).foregroundStyle(look.muted)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                TimelineView(.animation(minimumInterval: reduce ? 1.0 / 15 : 1.0 / 60, paused: !playing)) { ctx in
                    Canvas { gc, size in
                        draw(gc, size: size, bands: sample(ctx.date), style: v)
                    }
                }
            }
        }
        .opacity(Double(v.opacity))
        .accessibilityHidden(true)
    }

    private func sample(_ date: Date) -> [Float] {
        let v = Look.shared.viz
        let smoothing = (Look.shared.reduceMotion || reduce) ? max(v.smoothing, 0.85) : v.smoothing
        return store.core.visualizerBands(bars: v.bars, sensitivity: v.sensitivity, smoothing: smoothing)
    }

    private func draw(_ gc: GraphicsContext, size: CGSize, bands: [Float], style: VizStyleModel) {
        let look = Look.shared
        let gradient = Gradient(colors: [look.vizColor, look.vizColor2])
        let shading = GraphicsContext.Shading.linearGradient(gradient, startPoint: CGPoint(x: 0, y: size.height), endPoint: .zero)
        guard !bands.isEmpty else {
            // At rest: a thin line.
            gc.fill(Path(CGRect(x: 0, y: size.height - 1, width: size.width, height: 1)), with: .color(look.vizColor.opacity(0.4)))
            return
        }
        let n = CGFloat(bands.count)
        let w = size.width / n
        var path = Path()
        switch style.style {
        case "wave":
            for (i, b) in bands.enumerated() {
                let x = (CGFloat(i) + 0.5) * w
                let y = size.height * (1 - CGFloat(b) * 0.9)
                if i == 0 { path.move(to: CGPoint(x: x, y: y)) } else { path.addLine(to: CGPoint(x: x, y: y)) }
            }
            if style.glow {
                var glow = gc
                glow.addFilter(.blur(radius: 5))
                glow.opacity = 0.6
                glow.stroke(path, with: shading, style: StrokeStyle(lineWidth: 6, lineCap: .round, lineJoin: .round))
            }
            gc.stroke(path, with: shading, style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
            return
        case "mirror":
            for (i, b) in bands.enumerated() {
                let h = max(CGFloat(b) * size.height * 0.5, 1)
                path.addRoundedRect(in: CGRect(x: CGFloat(i) * w + w * 0.18, y: size.height / 2 - h, width: w * 0.64, height: h * 2), cornerSize: CGSize(width: w * 0.3, height: w * 0.3))
            }
        default:
            for (i, b) in bands.enumerated() {
                let h = max(CGFloat(b) * size.height, 1)
                path.addRoundedRect(in: CGRect(x: CGFloat(i) * w + w * 0.18, y: size.height - h, width: w * 0.64, height: h), cornerSize: CGSize(width: w * 0.3, height: w * 0.3))
            }
        }
        if style.glow {
            var glow = gc
            glow.addFilter(.blur(radius: 6))
            glow.opacity = 0.55
            glow.fill(path, with: shading)
        }
        gc.fill(path, with: shading)
    }
}
