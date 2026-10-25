import SwiftUI

/// The theme manager (desktop themepanel.rs, ⌘T): every theme.toml value Glass
/// draws with, shown live as you change it and written into the file when you let
/// go (validated by the core; comments and other keys stay). A preview of the UI
/// sits at the top; at the smaller height the app stays usable behind the panel, so
/// any screen can be watched while editing. Album colors live here too.
struct ThemePanel: View {
    @Environment(Store.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var tab: Tab = .colors

    enum Tab: String, CaseIterable {
        case colors, album, type, lyrics, visualizer, file
    }

    var body: some View {
        let look = Look.shared
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ThemePreview()
                    if store.theme?.error != nil {
                        Text("Your theme has a mistake, so the last good one is shown. Change anything here, or reset it, to fix it.").gFont(11.5).foregroundStyle(look.sub)
                    }
                    ScrollView(.horizontal) {
                        HStack(spacing: 6) {
                            ForEach(Tab.allCases, id: \.self) { t in Chip(title: t.rawValue, on: tab == t) { withAnimation(.easeOut(duration: 0.2)) { tab = t } } }
                        }
                    }
                    .scrollIndicators(.hidden)
                    Group {
                        switch tab {
                        case .colors: ColorsTab()
                        case .album: AlbumTab()
                        case .type: TypeTab()
                        case .lyrics: LyricsTab()
                        case .visualizer: VisualizerTab()
                        case .file: FileTab()
                        }
                    }
                    .transition(.opacity)
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 30)
            }
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("theme")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() }.gFont(14) } }
        }
        .presentationDetents([.fraction(0.62), .large])
        // At the smaller height, the app behind stays usable: open Now Playing, a page…
        .presentationBackgroundInteraction(.enabled(upThrough: .fraction(0.62)))
        .presentationBackground(look.overlay)
        .foregroundStyle(look.foreground)
        .tint(look.accent)
    }
}

// MARK: preview

/// A slice of Glass in the current theme: the player bar, a row, buttons and chips,
/// lyrics lines and the visualizer's colors. Redraws as values change.
private struct ThemePreview: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        let item = store.playback?.item
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Art(url: item?.imageSmall, size: 44)
                VStack(alignment: .leading, spacing: 2) {
                    Text(item?.title ?? "Golden Hour").gFont(14).foregroundStyle(look.foreground).lineLimit(1)
                    Text(item.map { Fmt.artists($0.artists) } ?? "Aurora Lane").gFont(12).foregroundStyle(look.sub).lineLimit(1)
                }
                Spacer()
                Image(systemName: "heart.fill").foregroundStyle(look.accent)
                Image(systemName: "pause.fill").foregroundStyle(look.foreground)
            }
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Rectangle().fill(look.track)
                    Rectangle().fill(look.progress).frame(width: g.size.width * 0.42)
                }
            }
            .frame(height: 2)
            HStack(spacing: 8) {
                Text("[ play ]").gFont(13).foregroundStyle(look.accent)
                Text("[ shuffle ]").gFont(13).foregroundStyle(look.foreground)
                Text("on").gFont(12).foregroundStyle(look.accent).padding(.horizontal, 10).padding(.vertical, 4)
                    .background(Capsule().fill(look.selection)).overlay(Capsule().strokeBorder(look.accent.opacity(0.8)))
                Text("off").gFont(12).foregroundStyle(look.sub).padding(.horizontal, 10).padding(.vertical, 4)
                    .overlay(Capsule().strokeBorder(look.divider))
                Spacer()
            }
            VStack(alignment: .leading, spacing: 2) {
                Text("we let the evening carry us").gFont(13 * CGFloat(look.lyrics.size) / 0.8).foregroundStyle(look.lyricsOther)
                (Text("drifting softly ").foregroundStyle(look.lyricsCurrent) + Text("past the moon").foregroundStyle(look.lyricsHighlight))
                    .gFont(14 * CGFloat(look.lyrics.size) / 0.8, look.lyrics.bold ? .bold : .regular)
            }
            HStack(spacing: 3) {
                ForEach(0..<24, id: \.self) { i in
                    let h = [0.3, 0.6, 0.8, 0.5, 0.9, 0.4, 0.7, 0.55][i % 8]
                    RoundedRectangle(cornerRadius: 1.5)
                        .fill(LinearGradient(colors: [look.vizColor, look.vizColor2], startPoint: .bottom, endPoint: .top))
                        .frame(width: 4, height: 26 * h)
                        .opacity(Double(look.viz.opacity))
                }
            }
            .frame(height: 26, alignment: .bottom)
        }
        .padding(14)
        .background(GlassBackground(wash: look.albumEverywhere, bloomFrom: UnitPoint(x: 0.1, y: 0.1)).clipShape(RoundedRectangle(cornerRadius: look.radius, style: .continuous)))
        .overlay(RoundedRectangle(cornerRadius: look.radius, style: .continuous).strokeBorder(look.divider))
        .padding(.top, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Preview of the theme")
    }
}

// MARK: editing

/// Writes a value (the core checks it; a refusal shows as a toast).
@MainActor private func setValue(_ store: Store, _ key: String, _ value: String) { store.core.setThemeValue(key: key, value: value) }
/// Shows a value without writing it (while dragging).
@MainActor private func previewValue(_ store: Store, _ key: String, _ value: String) { store.core.previewThemeValue(key: key, value: value) }

private struct ColorsTab: View {
    @Environment(Store.self) private var store
    @State private var more = false

    static let presets: [(String, [String])] = [
        ("glass", ["#0F1115", "#E9E4DC", "#4F83A3", "#090A0B"]),
        ("midnight", ["#0B1020", "#DCE3F2", "#7AA2F7", "#05070F"]),
        ("ember", ["#160F0D", "#F1E4D8", "#D89B7A", "#0A0605"]),
        ("moss", ["#0E1411", "#E2EAD9", "#8FB573", "#060A07"]),
        ("rose", ["#150E14", "#F0E2EA", "#D27A9E", "#0A0609"]),
    ]

    var body: some View {
        let look = Look.shared
        if let t = store.theme {
            VStack(alignment: .leading, spacing: 12) {
                Caps(text: "presets")
                ScrollView(.horizontal) {
                    HStack(spacing: 10) {
                        ForEach(Self.presets, id: \.0) { name, c in
                            Button {
                                for (k, v) in zip(["theme.background", "theme.foreground", "theme.accent", "theme.border"], c) { setValue(store, k, v) }
                            } label: {
                                VStack(spacing: 4) {
                                    ZStack(alignment: .bottomTrailing) {
                                        RoundedRectangle(cornerRadius: 8).fill(Color(css: c[0]) ?? .black).frame(width: 64, height: 42)
                                            .overlay(Text("Aa").gFont(13).foregroundStyle(Color(css: c[1]) ?? .white))
                                        Circle().fill(Color(css: c[2]) ?? .blue).frame(width: 12, height: 12).padding(5)
                                    }
                                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(t.accent.uppercased() == c[2] && t.background.uppercased() == c[0] ? look.accent : look.divider, lineWidth: 1.5))
                                    Text(name).gFont(11).foregroundStyle(look.sub)
                                }
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Preset \(name)")
                        }
                    }
                }
                .scrollIndicators(.hidden)
                Caps(text: "colors")
                ColorRow(label: "background", key: "theme.background", hex: t.background)
                ColorRow(label: "foreground", key: "theme.foreground", hex: t.foreground)
                ColorRow(label: "accent", key: "theme.accent", hex: t.accent)
                if look.albumEverywhere {
                    Text("Album colors apply everywhere, so while music plays the cover's accent shows instead of this one (album tab).").gFont(11).foregroundStyle(look.muted)
                }
                if !t.palette.isEmpty {
                    HStack(spacing: 8) {
                        Text("palette").gFont(12).foregroundStyle(look.sub).frame(width: 96, alignment: .leading)
                        ForEach(t.palette, id: \.self) { hex in
                            Button { setValue(store, "theme.accent", hex) } label: {
                                Circle().fill(Color(css: hex) ?? .clear).frame(width: 26, height: 26)
                                    .overlay(Circle().strokeBorder(t.accent.uppercased() == hex.uppercased() ? look.foreground : look.divider, lineWidth: 1.5))
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel("Use \(hex) as the accent")
                        }
                    }
                }
                SliderRow(label: "opacity", key: "theme.opacity", value: Double(t.opacity), range: 0...1, step: 0.01)
                Text("How much of the tint covers the depth behind it.").gFont(11).foregroundStyle(look.muted)
                DisclosureGroup(isExpanded: $more) {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("Worked out from the colors above unless set here.").gFont(11).foregroundStyle(look.muted)
                        ColorRow(label: "surface", key: "colors.surface", hex: t.surface)
                        ColorRow(label: "raised", key: "colors.surface_raised", hex: t.surfaceRaised)
                        ColorRow(label: "secondary text", key: "colors.text_secondary", hex: t.textSecondary)
                        ColorRow(label: "muted text", key: "colors.text_muted", hex: t.textMuted)
                        ColorRow(label: "selection", key: "colors.selection", hex: t.selection)
                        ColorRow(label: "track", key: "colors.player_track", hex: t.playerTrack)
                        ColorRow(label: "player", key: "colors.player_background", hex: t.playerBackground)
                        BracketButton(title: "work these out again") { store.core.resetTheme(table: "colors") }
                    }
                    .padding(.top, 8)
                } label: {
                    Text("more colors").gFont(13)
                }
                BracketButton(title: "reset colors") { store.core.resetTheme(table: "theme") }
            }
        }
    }
}

/// Album colors: on or off (Settings ▸ Display), and where they apply.
private struct AlbumTab: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        let on = store.settings?.albumColors ?? false
        VStack(alignment: .leading, spacing: 12) {
            Toggle(isOn: Binding(get: { on }, set: { v in withAnimation(look.colorAnimation) { store.editSettings { $0.albumColors = v } } })) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Match colors to the album cover").gFont(14)
                    Text("The accent, player and lyrics take their colors from the cover of what's playing, and its colors wash in behind.").gFont(11.5).foregroundStyle(look.sub)
                }
            }
            .tint(look.accent)
            if on {
                Text("where").gFont(12).foregroundStyle(look.sub)
                HStack(spacing: 8) {
                    Chip(title: "Now Playing", on: look.albumScope == .nowPlaying) { withAnimation(look.colorAnimation) { look.albumScope = .nowPlaying } }
                    Chip(title: "everywhere", on: look.albumScope == .everywhere) { withAnimation(look.colorAnimation) { look.albumScope = .everywhere } }
                }
                Text(look.albumScope == .everywhere
                    ? "The whole app takes the album's colors: every page, the player bar, Now Playing. When the song changes, the new colors bloom out from the artwork over 1.4 s."
                    : "Now Playing and the player bar take the album's colors, as on the desktop; the rest keeps the theme's. When the song changes, the new colors bloom out from the artwork over 1.4 s.")
                    .gFont(11.5).foregroundStyle(look.muted)
                if let a = look.album {
                    HStack(spacing: 8) {
                        Text("now").gFont(12).foregroundStyle(look.sub).frame(width: 44, alignment: .leading)
                        ForEach([a.accent, a.playerProgress, a.lyricsCurrent, a.lyricsHighlight] + a.wash, id: \.self) { hex in
                            Circle().fill(Color(css: hex) ?? .clear).frame(width: 22, height: 22).overlay(Circle().strokeBorder(look.divider))
                        }
                    }
                    .accessibilityHidden(true)
                } else {
                    Text("The colors show once something is playing.").gFont(11.5).foregroundStyle(look.muted)
                }
                if look.reduceMotion || UIAccessibility.isReduceMotionEnabled {
                    Text("Reduce Motion is on, so colors change in place instead of blooming.").gFont(11.5).foregroundStyle(look.muted)
                }
            }
        }
    }
}

private struct TypeTab: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        if let t = store.theme {
            VStack(alignment: .leading, spacing: 12) {
                Caps(text: "text & layout")
                SliderRow(label: "text size", key: "font.size", value: Double(t.fontSize), range: 10...20, step: 0.5)
                Text("Scales with your iPhone's text size too.").gFont(11).foregroundStyle(look.muted)
                SliderRow(label: "corners", key: "layout.radius", value: Double(t.radius), range: 0...24, step: 1)
                SliderRow(label: "spacing", key: "layout.spacing", value: Double(t.spacing), range: 4...14, step: 0.5)
                ChoiceRow(label: "motion", options: [("system", "system"), ("true", "reduced"), ("false", "full")], selected: t.motionSetting) { setValue(store, "motion.reduce", $0) }
                Text("\"system\" follows Settings › Accessibility › Motion.").gFont(11).foregroundStyle(look.muted)
                BracketButton(title: "reset") {
                    store.core.resetTheme(table: "font")
                    store.core.resetTheme(table: "layout")
                    store.core.resetTheme(table: "motion")
                }
            }
        }
    }
}

private struct LyricsTab: View {
    @Environment(Store.self) private var store
    var body: some View {
        if let l = store.theme?.lyrics {
            VStack(alignment: .leading, spacing: 12) {
                Caps(text: "lyrics")
                ColorRow(label: "current", key: "lyrics.current", hex: l.current)
                ColorRow(label: "other", key: "lyrics.other", hex: l.other)
                ColorRow(label: "highlight", key: "lyrics.highlight", hex: l.highlight)
                SliderRow(label: "size", key: "lyrics.size", value: Double(l.size), range: 0.5...2, step: 0.05)
                ChoiceRow(label: "align", options: ["left", "center", "right"].map { ($0, $0) }, selected: l.align) { setValue(store, "lyrics.align", $0) }
                ChoiceRow(label: "position", options: ["top", "upper", "middle"].map { ($0, $0) }, selected: l.position) { setValue(store, "lyrics.position", $0) }
                SliderRow(label: "spacing", key: "lyrics.spacing", value: Double(l.spacing), range: 0.5...2.5, step: 0.05)
                SliderRow(label: "fade", key: "lyrics.fade", value: Double(l.fade), range: 0...1, step: 0.01)
                ChoiceRow(label: "animation", options: ["glide", "fade", "pop", "none"].map { ($0, $0) }, selected: l.animation) { setValue(store, "lyrics.animation", $0) }
                SliderRow(label: "speed", key: "lyrics.speed", value: Double(l.speed), range: 0.25...3, step: 0.05)
                Toggle(isOn: Binding(get: { l.bold }, set: { setValue(store, "lyrics.bold", $0 ? "true" : "false") })) { Text("bold line").gFont(13) }
                Text("With album colors on, the cover's colors replace these three.").gFont(11).foregroundStyle(Look.shared.muted)
                BracketButton(title: "reset lyrics") { store.core.resetTheme(table: "lyrics") }
            }
        }
    }
}

private struct VisualizerTab: View {
    @Environment(Store.self) private var store
    var body: some View {
        if let v = store.theme?.visualizer {
            VStack(alignment: .leading, spacing: 12) {
                Caps(text: "visualizer")
                Toggle(isOn: Binding(get: { store.settings?.visualizer ?? false }, set: { on in store.editSettings { $0.visualizer = on } })) { Text("show under the lyrics").gFont(13) }
                ChoiceRow(label: "style", options: ["bars", "mirror", "wave"].map { ($0, $0) }, selected: v.style) { setValue(store, "visualizer.style", $0) }
                ColorRow(label: "color", key: "visualizer.color", hex: v.color)
                ColorRow(label: "second color", key: "visualizer.color2", hex: v.color2)
                SliderRow(label: "opacity", key: "visualizer.opacity", value: Double(v.opacity), range: 0.1...1, step: 0.01)
                SliderRow(label: "bars", key: "visualizer.bars", value: Double(v.bars), range: 12...128, step: 1)
                SliderRow(label: "height", key: "visualizer.height", value: Double(v.height), range: 0.15...0.6, step: 0.01)
                SliderRow(label: "sensitivity", key: "visualizer.sensitivity", value: Double(v.sensitivity), range: 0.25...4, step: 0.05)
                SliderRow(label: "smoothing", key: "visualizer.smoothing", value: Double(v.smoothing), range: 0...0.95, step: 0.01)
                Toggle(isOn: Binding(get: { v.glow }, set: { setValue(store, "visualizer.glow", $0 ? "true" : "false") })) { Text("glow").gFont(13) }
                VisualizerView().frame(height: 80)
                BracketButton(title: "reset visualizer") { store.core.resetTheme(table: "visualizer") }
            }
        }
    }
}

private struct FileTab: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 10) {
            Text("Theme file").gFont(15)
            Text("Everything here is saved as you go. Export your theme to share it or use it on another device; Import uses one.").gFont(11.5).foregroundStyle(look.muted)
            FileTransferButtons(which: .theme)
        }
    }
}

// MARK: rows

/// A color: the swatch opens the system's picker (shown live, saved a moment after
/// the last change); the hex can be typed (with or without #).
struct ColorRow: View {
    @Environment(Store.self) private var store
    let label: String
    let key: String
    let hex: String
    @State private var text = ""
    @State private var save: Task<Void, Never>?

    var body: some View {
        let look = Look.shared
        HStack {
            Text(label).gFont(13)
            Spacer()
            ColorPicker(label, selection: Binding(get: { Color(css: hex) ?? .clear }, set: { c in picked(c.hexString) }), supportsOpacity: true).labelsHidden()
            TextField("#RRGGBB", text: $text).gFont(12.5).frame(width: 104).multilineTextAlignment(.trailing)
                .textInputAutocapitalization(.characters).autocorrectionDisabled()
                .submitLabel(.done)
                .onSubmit { commitTyped() }
                .padding(.horizontal, 6).frame(height: 32)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(look.surface))
                .accessibilityLabel("\(label), hex")
        }
        .onAppear { text = hex }
        .onChange(of: hex) { _, h in text = h }
    }

    private func picked(_ h: String) {
        text = h
        previewValue(store, key, h)
        // The picker reports every movement: the file is written once it settles.
        save?.cancel()
        save = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(500))
            if !Task.isCancelled { setValue(store, key, h) }
        }
    }

    private func commitTyped() {
        var t = text.trimmingCharacters(in: .whitespaces)
        if !t.isEmpty, !t.hasPrefix("#") { t = "#" + t }
        setValue(store, key, t)
    }
}

/// A number: shown live while dragging, written when let go (as the desktop panel does).
struct SliderRow: View {
    @Environment(Store.self) private var store
    let label: String
    let key: String
    let value: Double
    let range: ClosedRange<Double>
    let step: Double
    @State private var v: Double?

    private func raw(_ x: Double) -> String { step >= 1 ? String(Int(x.rounded())) : String(format: "%.2f", x) }

    var body: some View {
        HStack {
            Text(label).gFont(13).frame(width: 96, alignment: .leading)
            Slider(value: Binding(get: { v ?? value }, set: { x in v = x; previewValue(store, key, raw(x)) }), in: range, step: step) { editing in
                if !editing, let v { setValue(store, key, raw(v)); self.v = nil }
            }
            .accessibilityLabel(label)
            Text(raw(v ?? value)).gFont(11.5).foregroundStyle(Look.shared.sub).frame(width: 44).monospacedDigit()
        }
    }
}

struct ChoiceRow: View {
    let label: String
    let options: [(String, String)]
    let selected: String
    let pick: (String) -> Void
    var body: some View {
        HStack {
            Text(label).gFont(13).frame(width: 96, alignment: .leading)
            ChipPicker(options: options, selected: selected, pick: pick)
        }
    }
}

extension ThemeModel {
    /// theme.toml's [motion] reduce as written: "system", "true" or "false".
    var motionSetting: String { motionReduce }
}

extension Color {
    /// `#RRGGBB` (or `#RRGGBBAA` when not opaque), for theme.toml.
    var hexString: String {
        let u = UIColor(self)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        u.getRed(&r, green: &g, blue: &b, alpha: &a)
        let c = { (x: CGFloat) in Int((min(max(x, 0), 1) * 255).rounded()) }
        return a >= 0.999 ? String(format: "#%02X%02X%02X", c(r), c(g), c(b)) : String(format: "#%02X%02X%02X%02X", c(r), c(g), c(b), c(a))
    }
}
