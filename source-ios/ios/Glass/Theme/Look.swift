import SwiftUI

/// Glass's design tokens. The defaults are theme.toml's (src/theme_config.rs):
/// background #0F1115, foreground #E9E4DC, accent #4F83A3, border #090A0B,
/// opacity 0.45, radius 10, spacing 8, and the derived [colors]. Observable so
/// a theme change (theme.toml, the theme panel, album colors) redraws.
@Observable
@MainActor
final class Look {
    static let shared = Look()

    var background = Color(hex: 0x0F1115)
    var foreground = Color(hex: 0xE9E4DC)
    /// theme.toml's accent (`accent` below is the one in effect, which album colors can replace).
    var themeAccent = Color(hex: 0x4F83A3)
    var border = Color(hex: 0x090A0B)
    var opacity: Double = 0.45
    var radius: CGFloat = 10
    var spacing: CGFloat = 8
    /// The UI's type size relative to the desktop's 13 pt base.
    var scale: CGFloat = 1.0
    // theme.toml's [colors] (derived from the four base colors unless the file sets them).
    var surface = Color(hex: 0xE9E4DC, alpha: 0.04)
    var raised = Color(hex: 0xE9E4DC, alpha: 0.08)
    var overlay = Color(hex: 0x0F1115, alpha: 0.94)
    var sub = Color(hex: 0xE9E4DC, alpha: 0.80)
    var muted = Color(hex: 0xE9E4DC, alpha: 0.62)
    var onAccent = Color(hex: 0xE9E4DC)
    var hover = Color(hex: 0xE9E4DC, alpha: 0.06)
    var focus = Color(hex: 0x4F83A3)
    var themeSelection = Color(hex: 0x4F83A3, alpha: 0.16)
    var themePlayerBackground = Color(hex: 0x0F1115, alpha: 0.57)
    var themeProgress = Color(hex: 0x4F83A3)
    var themeTrack = Color(hex: 0xE9E4DC, alpha: 0.25)
    /// Album colors: the playing cover's palette applied to the theme (nil: off, or not ready).
    var album: AlbumColorsModel?
    var albumWash: [Color] { album.map { $0.wash.compactMap { Color(css: $0) } } ?? [] }
    /// Where album colors apply: Now Playing and the player bar (the desktop's way), or
    /// the whole app. Kept on this device.
    var albumScope: AlbumScope = AlbumScope(rawValue: UserDefaults.standard.string(forKey: AlbumScope.key) ?? "") ?? .nowPlaying {
        didSet { UserDefaults.standard.set(albumScope.rawValue, forKey: AlbumScope.key) }
    }
    /// Album colors are on and apply everywhere right now.
    var albumEverywhere: Bool { album != nil && albumScope == .everywhere }
    /// theme.toml's [lyrics] and [visualizer], and [motion] (resolved).
    var lyrics = LyricsStyleModel(current: "#6D3A78", other: "#80D0D8", highlight: "#4F83A3", size: 0.8, align: "center", position: "upper", spacing: 1.5, fade: 0.59, animation: "glide", speed: 0.75, bold: true)
    var viz = VizStyleModel(style: "mirror", color: "#4F83A3", color2: "#E9E4DCE6", opacity: 0.91, bars: 54, height: 0.26, sensitivity: 1.1, smoothing: 0.65, glow: true)
    var reduceMotion = false
    var palette: [String] = []

    var lyricsCurrent: Color { Color(css: album?.lyricsCurrent ?? lyrics.current) ?? foreground }
    var lyricsOther: Color { Color(css: album?.lyricsOther ?? lyrics.other) ?? sub }
    var lyricsHighlight: Color { Color(css: album?.lyricsHighlight ?? lyrics.highlight) ?? accent }
    var vizColor: Color { Color(css: album?.vizColor ?? viz.color) ?? accent }
    var vizColor2: Color { Color(css: album?.vizColor2 ?? viz.color2) ?? foreground }

    /// A theme from the core (theme.toml): the base colors, [colors], sizes and styles.
    func apply(_ t: ThemeModel) {
        background = Color(css: t.background) ?? background
        foreground = Color(css: t.foreground) ?? foreground
        themeAccent = Color(css: t.accent) ?? themeAccent
        border = Color(css: t.border) ?? border
        opacity = Double(t.opacity)
        radius = CGFloat(t.radius)
        spacing = CGFloat(t.spacing)
        // The desktop's sizes scale from font.size (13 pt by default).
        scale = CGFloat(t.fontSize) / 13
        surface = Color(css: t.surface) ?? surface
        raised = Color(css: t.surfaceRaised) ?? raised
        overlay = Color(css: t.overlay) ?? overlay
        sub = Color(css: t.textSecondary) ?? sub
        muted = Color(css: t.textMuted) ?? muted
        onAccent = Color(css: t.onAccent) ?? onAccent
        hover = Color(css: t.hover) ?? hover
        focus = Color(css: t.focus) ?? focus
        themeSelection = Color(css: t.selection) ?? themeSelection
        themePlayerBackground = Color(css: t.playerBackground) ?? themePlayerBackground
        themeProgress = Color(css: t.playerProgress) ?? themeProgress
        themeTrack = Color(css: t.playerTrack) ?? themeTrack
        lyrics = t.lyrics
        viz = t.visualizer
        reduceMotion = t.reduceMotion
        palette = t.palette
    }

    /// The accent in effect: the album's when album colors apply everywhere.
    var accent: Color { albumEverywhere ? (album.flatMap { Color(css: $0.accent) } ?? themeAccent) : themeAccent }
    var accentNow: Color { accent }
    var selection: Color { albumEverywhere ? (album.flatMap { Color(css: $0.selection) } ?? themeSelection) : themeSelection }
    var playerBackground: Color { albumEverywhere ? (album.flatMap { Color(css: $0.playerBackground) } ?? themePlayerBackground) : themePlayerBackground }
    var progress: Color { albumEverywhere ? (album.flatMap { Color(css: $0.playerProgress) } ?? themeProgress) : themeProgress }
    var track: Color { albumEverywhere ? (album.flatMap { Color(css: $0.playerTrack) } ?? themeTrack) : themeTrack }
    var divider: Color { foreground.opacity(0.09) }
    var error: Color { Color(red: 240 / 255, green: 140 / 255, blue: 120 / 255) }
    var smallRadius: CGFloat { radius * 0.6 }
    /// The album colors' transition (the desktop's 1.4 s bloom), or none with Reduce Motion.
    var colorAnimation: Animation? { reduceMotion || UIAccessibility.isReduceMotionEnabled ? nil : .easeInOut(duration: 1.4) }
    func sp(_ n: CGFloat) -> CGFloat { spacing * n }
}

enum AlbumScope: String, CaseIterable {
    case nowPlaying, everywhere
    static let key = "albumColorsScope"
}

extension Color {
    init(hex: UInt32, alpha: Double = 1) {
        self.init(.sRGB, red: Double((hex >> 16) & 0xFF) / 255, green: Double((hex >> 8) & 0xFF) / 255, blue: Double(hex & 0xFF) / 255, opacity: alpha)
    }

    /// `#RGB`, `#RRGGBB` or `#RRGGBBAA` (theme.toml's color syntax).
    init?(css: String) {
        var s = css.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") { s.removeFirst() }
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6 || s.count == 8, let v = UInt64(s, radix: 16) else { return nil }
        if s.count == 8 {
            self.init(.sRGB, red: Double((v >> 24) & 0xFF) / 255, green: Double((v >> 16) & 0xFF) / 255, blue: Double((v >> 8) & 0xFF) / 255, opacity: Double(v & 0xFF) / 255)
        } else {
            self.init(hex: UInt32(v))
        }
    }
}

// MARK: accent in context

/// The accent views draw with: the theme's, or (on Now Playing and the player bar,
/// as on the desktop) the album's.
private struct GlassAccentKey: EnvironmentKey {
    static let defaultValue: Color? = nil
}

extension EnvironmentValues {
    var glassAccentOverride: Color? {
        get { self[GlassAccentKey.self] }
        set { self[GlassAccentKey.self] = newValue }
    }
}

/// Reads the accent for this place in the view tree.
@MainActor
@propertyWrapper
struct GlassAccent: DynamicProperty {
    @Environment(\.glassAccentOverride) private var override
    var wrappedValue: Color {
        if let override { return override }
        return Look.shared.accent
    }
}

extension View {
    /// Album colors here (Now Playing, the player bar), when there are any.
    func albumTinted() -> some View { modifier(AlbumTint()) }
}

struct AlbumTint: ViewModifier {
    func body(content: Content) -> some View {
        let a = Look.shared.album.flatMap { Color(css: $0.accent) }
        content.environment(\.glassAccentOverride, a).tint(a ?? Look.shared.accent)
    }
}

// MARK: type

/// Glass is set in SF Mono (the desktop's default UI and lyrics font). On iOS that's
/// the system's monospaced design, which ships with the OS and scales with Dynamic Type.
struct GFont: ViewModifier {
    let size: CGFloat
    var weight: Font.Weight = .regular
    @ScaledMetric(relativeTo: .body) private var unit: CGFloat = 1

    func body(content: Content) -> some View {
        content.font(.system(size: size * Look.shared.scale * unit, weight: weight, design: .monospaced))
    }
}

extension View {
    func gFont(_ size: CGFloat, _ weight: Font.Weight = .regular) -> some View {
        modifier(GFont(size: size, weight: weight))
    }
}

// MARK: background

/// The pane of smoked glass behind everything. The desktop's window is see-through
/// over the wallpaper; an iOS app can't blur other apps, so the depth is made in
/// the app's own scene: the theme's tint over a dim, slowly varying field, with the
/// album's colors washing in on Now Playing (or everywhere, by choice).
struct GlassBackground: View {
    var wash = false
    /// Where a new song's colors bloom from: the artwork (top left on Now Playing,
    /// the player bar's cover at the bottom elsewhere).
    var bloomFrom = UnitPoint(x: 0.2, y: 0.25)

    var body: some View {
        let look = Look.shared
        ZStack {
            Color.black
            // What the glass sits over: a dim, cool field, like the desktop demo's wallpaper behind the window.
            LinearGradient(colors: [Color(hex: 0x232A3A), Color(hex: 0x10131A), Color(hex: 0x1B2130)], startPoint: .topLeading, endPoint: .bottomTrailing)
                .opacity(1 - look.opacity)
            look.background.opacity(look.opacity)
            if wash, let a = look.album {
                // A new song's colors bloom out from the artwork over 1.4 s, as on the
                // desktop (smoothstep ≈ ease in-out); with Reduce Motion they change in place.
                MeshWash(colors: look.albumWash)
                    .id(a.cover)
                    .transition(look.colorAnimation == nil ? .opacity : .asymmetric(insertion: .scale(scale: 0.15, anchor: bloomFrom).combined(with: .opacity), removal: .opacity))
            }
        }
        .ignoresSafeArea()
    }
}

/// A soft three-color wash (album colors).
struct MeshWash: View {
    let colors: [Color]
    var body: some View {
        GeometryReader { g in
            ZStack {
                ForEach(Array(colors.prefix(3).enumerated()), id: \.offset) { i, c in
                    Circle()
                        .fill(c)
                        .frame(width: g.size.width * 1.1, height: g.size.width * 1.1)
                        .offset(x: [-0.3, 0.35, -0.1][i] * g.size.width, y: [-0.25, 0.1, 0.45][i] * g.size.height)
                        .blur(radius: 90)
                        .opacity(0.55)
                }
            }
            .frame(width: g.size.width, height: g.size.height)
        }
        .allowsHitTesting(false)
    }
}
