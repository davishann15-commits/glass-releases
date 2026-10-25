import SwiftUI
import UIKit
import ImageIO

// MARK: images

/// Covers and artist images: Spotify's CDN, a local file's embedded art
/// (`localcover://<path>`, read by the core), or the demo's gradients
/// (`glass-demo:<n>`). Each image is downloaded once (kept on disk across launches:
/// Spotify's image URLs never change content) and decoded off the main thread,
/// downsampled to a few standard sizes. Any size already decoded stands in at once
/// while a larger one is made, and covers about to be needed are fetched ahead.
@MainActor
final class ImageCache {
    static let shared = ImageCache()
    private let images = NSCache<NSString, UIImage>()
    private let bytes = NSCache<NSString, NSData>()
    private var decodes: [String: Task<UIImage?, Never>] = [:]
    private var downloads: [String: Task<Data?, Never>] = [:]
    /// The sizes decoded so far, per URL (for stand-ins).
    private var decoded: [String: Set<Int>] = [:]

    init() {
        images.totalCostLimit = 64 * 1024 * 1024
        bytes.totalCostLimit = 24 * 1024 * 1024
    }

    /// The images' own session: a large disk cache (Caches/Images), and cached copies
    /// used without asking the server again.
    nonisolated static let session: URLSession = {
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("Images", isDirectory: true)
        let c = URLSessionConfiguration.default
        c.urlCache = URLCache(memoryCapacity: 8 * 1024 * 1024, diskCapacity: 300 * 1024 * 1024, directory: dir)
        c.requestCachePolicy = .returnCacheDataElseLoad
        c.httpMaximumConnectionsPerHost = 6
        c.timeoutIntervalForRequest = 20
        return URLSession(configuration: c)
    }()

    /// Decoded sizes are rounded up to these, so places of similar size share one.
    nonisolated static func bucket(_ px: CGFloat) -> Int {
        for b in [96, 192, 384, 768] where CGFloat(b) >= px { return b }
        return 1280
    }

    private func key(_ url: String, _ b: Int) -> NSString { "\(url)@\(b)" as NSString }

    /// This image at this size, if it's decoded already.
    func cached(_ url: String, px: CGFloat) -> UIImage? { images.object(forKey: key(url, Self.bucket(px))) }

    /// The largest size of this image decoded so far (shown while the right one loads).
    func best(_ url: String) -> UIImage? {
        for b in (decoded[url] ?? []).sorted(by: >) {
            if let i = images.object(forKey: key(url, b)) { return i }
        }
        return nil
    }

    func load(_ url: String, px: CGFloat, priority: TaskPriority = .userInitiated) async -> UIImage? {
        let b = Self.bucket(px)
        let k = key(url, b)
        if let i = images.object(forKey: k) { return i }
        let dk = k as String
        if let t = decodes[dk] { return await t.value }
        let t = Task(priority: priority) { () -> UIImage? in
            if url.hasPrefix("glass-demo:") { return ImageCache.demo(Int(url.dropFirst("glass-demo:".count)) ?? 0) }
            guard let data = await self.data(url, priority: priority) else { return nil }
            return await Task.detached(priority: priority) { ImageCache.downsample(data, px: CGFloat(b)) }.value
        }
        decodes[dk] = t
        let img = await t.value
        decodes[dk] = nil
        if let img {
            images.setObject(img, forKey: k, cost: Int(img.size.width * img.size.height * img.scale * img.scale * 4))
            decoded[url, default: []].insert(b)
        }
        return img
    }

    /// Fetches covers ahead (the next songs'), at the sizes they'll be shown.
    func prefetch(_ url: String?, px: CGFloat) {
        guard let url, !url.isEmpty, cached(url, px: px) == nil else { return }
        Task { _ = await load(url, px: px, priority: .utility) }
    }

    /// The file's bytes: memory, then the session's disk cache, then the network; one
    /// download per URL however many sizes want it.
    private func data(_ url: String, priority: TaskPriority) async -> Data? {
        if let d = bytes.object(forKey: url as NSString) { return d as Data }
        if let t = downloads[url] { return await t.value }
        let t = Task.detached(priority: priority) { () -> Data? in
            if url.hasPrefix("localcover://") {
                return localCover(path: String(url.dropFirst("localcover://".count))).map { Data($0) }
            }
            guard let u = URL(string: url) else { return nil }
            return try? await ImageCache.session.data(from: u).0
        }
        downloads[url] = t
        let d = await t.value
        downloads[url] = nil
        if let d { bytes.setObject(d as NSData, forKey: url as NSString, cost: d.count) }
        return d
    }

    nonisolated static func downsample(_ data: Data, px: CGFloat) -> UIImage? {
        guard let src = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
        let opts: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceShouldCacheImmediately: true, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: max(px, 64)]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(src, 0, opts as CFDictionary) else { return nil }
        return UIImage(cgImage: cg)
    }

    /// The desktop demo's covers: a diagonal blend of two palette colors.
    nonisolated static func demo(_ n: Int) -> UIImage {
        let palette: [(CGFloat, CGFloat, CGFloat)] = [(236, 72, 153), (99, 102, 241), (16, 185, 129), (245, 158, 11), (14, 165, 233), (168, 85, 247), (239, 68, 68), (34, 197, 94)]
        let a = palette[n % palette.count], b = palette[(n * 3 + 1) % palette.count]
        let r = UIGraphicsImageRenderer(size: CGSize(width: 128, height: 128))
        return r.image { ctx in
            let colors = [UIColor(red: a.0 / 255, green: a.1 / 255, blue: a.2 / 255, alpha: 1).cgColor, UIColor(red: b.0 / 255, green: b.1 / 255, blue: b.2 / 255, alpha: 1).cgColor]
            let g = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors as CFArray, locations: [0, 1])!
            ctx.cgContext.drawLinearGradient(g, start: .zero, end: CGPoint(x: 128, y: 128), options: [])
        }
    }
}

/// A cover, square or round, with Glass's placeholder while it loads. When the cover
/// changes (a new song), the new one crossfades in over the old, as on the desktop;
/// a smaller copy (`fallback`, or this cover at another size) shows at once while the
/// sharp one is made.
struct Art: View {
    let url: String?
    var size: CGFloat? = nil
    var round = false
    var radius: CGFloat? = nil
    var placeholder: String = "music.note"
    /// Fill the frame it's given (an artist's banner) instead of staying square.
    var fill = false
    /// A smaller variant of the same cover, shown until this one is ready.
    var fallback: String? = nil
    @State private var image: UIImage?
    /// Which cover `image` is (a crossfade happens when this changes).
    @State private var shownFor = ""
    @Environment(\.displayScale) private var scale

    var body: some View {
        let look = Look.shared
        GeometryReader { g in
            ZStack {
                look.raised
                if let image {
                    Image(uiImage: image).resizable().scaledToFill()
                        .id(shownFor)
                        .transition(.opacity)
                } else {
                    Image(systemName: placeholder).font(.system(size: max(g.size.width * 0.3, 10), weight: .light)).foregroundStyle(look.muted)
                }
            }
            .frame(width: g.size.width, height: g.size.height)
            .clipShape(round ? AnyShape(Circle()) : AnyShape(RoundedRectangle(cornerRadius: radius ?? look.smallRadius, style: .continuous)))
            .task(id: "\(url ?? "")@\(ImageCache.bucket(max(g.size.width, 1) * scale))") {
                await show(url, px: max(g.size.width, 1) * scale)
            }
        }
        .frame(width: size, height: size)
        .modifier(SquareUnlessFill(fill: fill))
        .accessibilityHidden(true)
    }

    private func show(_ url: String?, px: CGFloat) async {
        let cache = ImageCache.shared
        guard let url, !url.isEmpty else {
            withAnimation(.easeInOut(duration: 0.3)) { image = nil; shownFor = "" }
            return
        }
        let fade = Look.shared.reduceMotion ? Animation.easeOut(duration: 0.15) : .easeInOut(duration: 0.45)
        func put(_ i: UIImage) {
            if shownFor == url {
                image = i  // the same cover, sharper: no fade
            } else {
                withAnimation(image == nil ? .easeOut(duration: 0.2) : fade) { image = i; shownFor = url }
            }
        }
        if let hit = cache.cached(url, px: px) { put(hit); return }
        if shownFor != url, let stand = cache.best(url) ?? fallback.flatMap({ cache.cached($0, px: px) ?? cache.best($0) }) { put(stand) }
        if let img = await cache.load(url, px: px), !Task.isCancelled { put(img) }
    }
}

// MARK: controls

private struct SquareUnlessFill: ViewModifier {
    let fill: Bool
    func body(content: Content) -> some View {
        if fill { content } else { content.aspectRatio(1, contentMode: .fit) }
    }
}

/// `[ play ]`: Glass's bracketed text buttons.
struct BracketButton: View {
    @GlassAccent private var accent
    let title: String
    var icon: String? = nil
    var primary = false
    let action: () -> Void

    var body: some View {
        let look = Look.shared
        Button(action: action) {
            HStack(spacing: 6) {
                Text("[").foregroundStyle(look.muted)
                if let icon { Image(systemName: icon).font(.system(size: 12 * look.scale, weight: .semibold)) }
                Text(title)
                Text("]").foregroundStyle(look.muted)
            }
            .gFont(13.5)
            .lineLimit(1)
            .fixedSize()
            .foregroundStyle(primary ? accent : look.foreground)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

/// An outlined pill (Follow, Rename, Next pick).
struct OutlineButton: View {
    @GlassAccent private var accent
    let title: String
    var on = false
    let action: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: action) {
            Text(title).gFont(13)
                .lineLimit(1)
                .fixedSize()
                .foregroundStyle(on ? accent : look.foreground)
                .padding(.horizontal, 14).padding(.vertical, 7)
                .overlay(Capsule().strokeBorder(on ? accent : look.foreground.opacity(0.35), lineWidth: 1))
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// A filter chip (lowercase, outlined; accent when selected).
struct Chip: View {
    @GlassAccent private var accent
    let title: String
    let on: Bool
    let action: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: action) {
            Text(title).gFont(12.5)
                .lineLimit(1)
                .fixedSize()
                .foregroundStyle(on ? accent : look.sub)
                .padding(.horizontal, 11).padding(.vertical, 6)
                .background(Capsule().fill(on ? look.selection : Color.clear))
                .overlay(Capsule().strokeBorder(on ? accent.opacity(0.8) : look.divider, lineWidth: 1))
                .frame(minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

struct IconButton: View {
    @GlassAccent private var accent
    let icon: String
    var size: CGFloat = 18
    var on = false
    let label: String
    let action: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: action) {
            Image(systemName: icon)
                .font(.system(size: size, weight: .regular))
                .foregroundStyle(on ? accent : look.sub)
                .frame(minWidth: 44, minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// The round play button (outlined accent ring, as the desktop draws it).
struct PlayCircle: View {
    @GlassAccent private var accent
    let playing: Bool
    var size: CGFloat = 52
    let action: () -> Void
    var body: some View {
        let look = Look.shared
        Button(action: action) {
            ZStack {
                Circle().fill(accent.opacity(0.12))
                Circle().strokeBorder(accent, lineWidth: 1.5)
                Image(systemName: playing ? "pause.fill" : "play.fill")
                    .font(.system(size: size * 0.32))
                    .foregroundStyle(accent)
                    .offset(x: playing ? 0 : size * 0.03)
            }
            .frame(width: size, height: size)
            .frame(minWidth: 44, minHeight: 44)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(playing ? "Pause" : "Play")
    }
}

// MARK: text

/// Small caps label ("COLLECTION", "ARTIST").
struct Caps: View {
    let text: String
    var color: Color? = nil
    var body: some View {
        Text(text.uppercased()).gFont(10.5).tracking(1.4).foregroundStyle(color ?? Look.shared.muted)
    }
}

struct Hairline: View {
    var body: some View { Rectangle().fill(Look.shared.divider).frame(height: 1) }
}

struct SectionTitle: View {
    let title: String
    var more: (() -> Void)? = nil
    var body: some View {
        let look = Look.shared
        HStack(alignment: .firstTextBaseline) {
            Text(title).gFont(19).foregroundStyle(look.foreground)
            Spacer()
            if let more {
                Button(action: more) { Text("show all ›").gFont(12.5).foregroundStyle(look.sub).frame(minHeight: 44) }.buttonStyle(.plain)
            }
        }
        .padding(.top, look.sp(2))
        .accessibilityAddTraits(.isHeader)
    }
}

/// "loading…"
struct LoadingNote: View {
    var body: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small).tint(Look.shared.muted)
            Text("loading…").gFont(13).foregroundStyle(Look.shared.muted)
        }
        .padding(.vertical, 24)
        .frame(maxWidth: .infinity)
    }
}

/// A title and body in place of content (desktop `widgets::unavailable`).
struct Unavailable: View {
    let title: String
    let detail: String
    var body: some View {
        VStack(spacing: 8) {
            Text(title).gFont(17).foregroundStyle(Look.shared.foreground)
            Text(detail).gFont(13).foregroundStyle(Look.shared.sub).multilineTextAlignment(.center)
        }
        .padding(.vertical, 40).padding(.horizontal, 24)
        .frame(maxWidth: .infinity)
    }
}

/// "couldn't load this" with the reason and [try again].
struct ErrorBox: View {
    let message: String
    let retry: () -> Void
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 8) {
            Text("couldn't load this").gFont(15).foregroundStyle(look.foreground)
            Text(message).gFont(12.5).foregroundStyle(look.sub)
            BracketButton(title: "try again", action: retry)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: look.radius).fill(look.surface))
        .overlay(RoundedRectangle(cornerRadius: look.radius).strokeBorder(look.divider))
    }
}

/// A panel of the glass (cards, the player bar).
struct Panel: ViewModifier {
    var radius: CGFloat? = nil
    func body(content: Content) -> some View {
        let look = Look.shared
        content
            .background(.ultraThinMaterial.opacity(0.85), in: RoundedRectangle(cornerRadius: radius ?? look.radius, style: .continuous))
            .background(RoundedRectangle(cornerRadius: radius ?? look.radius, style: .continuous).fill(look.playerBackground))
            .overlay(RoundedRectangle(cornerRadius: radius ?? look.radius, style: .continuous).strokeBorder(look.foreground.opacity(0.08), lineWidth: 1))
    }
}

extension View {
    func panel(radius: CGFloat? = nil) -> some View { modifier(Panel(radius: radius)) }
}

enum Fmt {
    /// 3:07, 1:02:03
    static func ms(_ ms: UInt64) -> String {
        let s = Int(ms / 1000)
        return s >= 3600 ? String(format: "%d:%02d:%02d", s / 3600, (s / 60) % 60, s % 60) : String(format: "%d:%02d", s / 60, s % 60)
    }
    static func artists(_ links: [Link]) -> String { links.map(\.name).joined(separator: ", ") }
}
