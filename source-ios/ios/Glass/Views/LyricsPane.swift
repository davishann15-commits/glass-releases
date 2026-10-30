import SwiftUI
import Translation

/// The lyrics view (desktop nowplaying.rs 334–1017): the lookup's state screens,
/// the picker, synced lyrics that follow the song (line, word or syllable — only as
/// precisely as the provider timed them), translation, timing and provenance.
/// Lookups run only while this is on screen (`lyrics_visible`).
struct LyricsPane: View {
    @Environment(Store.self) private var store
    let embedded: Bool

    var body: some View {
        let look = Look.shared
        Group {
            if let m = store.lyrics {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 0) {
                        Spacer(minLength: 0)
                        if m.state == .found { Credits(credits: m.credits) }
                        Text(m.header).gFont(12).foregroundStyle(look.muted).lineLimit(1)
                    }
                    // On its own line (full width, wraps): "from Japanese · Apple, on this
                    // device", "translating…", or what's needed and the button for it.
                    if let t = m.translation, m.state == .found, m.translationMode != .off { TranslationNote(t: t) }
                    if let picker = m.picker {
                        PickerView(m: m, picker: picker)
                    } else {
                        content(m)
                    }
                    if store.settings?.visualizer ?? false {
                        VisualizerView().frame(height: max(64, 300 * CGFloat(Look.shared.viz.height)))
                    }
                    if m.state == .found, m.picker == nil { LyricsFooter(m: m) }
                }
            } else {
                Unavailable(title: "nothing playing", detail: "Lyrics follow the song that's playing.")
            }
        }
        .onAppear { store.core.lyricsVisible(visible: true) }
        .onDisappear { store.core.lyricsVisible(visible: false) }
    }

    @ViewBuilder private func content(_ m: LyricsModel) -> some View {
        let look = Look.shared
        switch m.state {
        case .nothingPlaying:
            Unavailable(title: "nothing playing", detail: "Lyrics follow the song that's playing.")
        case let .loading(provider):
            Unavailable(title: "finding lyrics", detail: "Looking on \(provider)…")
        case .found:
            if m.timing == .plain { PlainLyrics(m: m, embedded: embedded) } else { SyncedLyrics(m: m, embedded: embedded) }
        case .instrumental:
            VStack(spacing: 10) {
                Text("♪  instrumental").gFont(20).foregroundStyle(look.foreground)
                Text("LRCLIB lists this recording as instrumental.").gFont(13).foregroundStyle(look.sub)
                BracketButton(title: "it has lyrics") { store.core.lyricsOpenPicker() }
            }
            .frame(maxWidth: .infinity).padding(.vertical, 30)
        case let .notFound(provider, others):
            VStack(spacing: 10) {
                Unavailable(title: "no lyrics found", detail: "\(provider) has no lyrics that match this recording." + (others.isEmpty ? "" : "\n" + others))
                HStack {
                    BracketButton(title: "search LRCLIB") { store.core.lyricsOpenPicker() }
                    BracketButton(title: "look again") { store.core.lyricsRetry() }
                }
            }
        case .ambiguous:
            VStack(alignment: .leading, spacing: 8) {
                Text("several versions match").gFont(17).foregroundStyle(look.foreground)
                Text("They have different lyrics. Pick the one that fits what's playing.").gFont(13).foregroundStyle(look.sub)
                CandidateList(candidates: m.candidates)
                HStack {
                    BracketButton(title: "search") { store.core.lyricsSearch(query: "\(m.title) \(m.artists)") }
                    BracketButton(title: "none of these") { store.core.lyricsNoneFit() }
                }
            }
        case let .unavailable(provider, reason):
            VStack(spacing: 10) {
                Unavailable(title: "\(provider) lyrics aren't available", detail: reason)
                BracketButton(title: "use LRCLIB (auto)") { store.core.setLyricsProvider(provider: "auto") }
            }
        case let .failed(provider, error):
            VStack(spacing: 10) {
                Unavailable(title: "couldn't load lyrics from \(provider)", detail: error)
                BracketButton(title: "try again") { store.core.lyricsRetry() }
            }
        }
    }
}

/// Where the lyrics are from, before the header: a link for services that ask for
/// credit (lrc.red, Spicy Lyrics' syncers), else the provider's name ("netease · ").
struct Credits: View {
    let credits: [CreditModel]
    var body: some View {
        let look = Look.shared
        ForEach(credits.indices, id: \.self) { i in
            credit(credits[i], look)
            Text(" · ").gFont(12).foregroundStyle(look.muted)
        }
    }

    @ViewBuilder private func credit(_ c: CreditModel, _ look: Look) -> some View {
        if let s = c.url, let u = URL(string: s) {
            SwiftUI.Link(destination: u) { Text(c.text).gFont(12).foregroundStyle(look.sub).lineLimit(1) }
                .accessibilityHint("Opens \(u.host() ?? "the source")")
        } else {
            Text(c.text).gFont(12).foregroundStyle(look.sub).lineLimit(1)
        }
    }
}

/// The current position in lyrics time: the player's position (or the seek bar
/// while it's dragged) plus the offsets (positive shows lyrics earlier).
@MainActor private func lyricsPosition(_ store: Store, _ m: LyricsModel, at date: Date) -> UInt64 {
    let base = store.scrubPosition(at: date) ?? store.position(at: date)
    let p = Int64(base) + m.globalOffsetMs + m.songOffsetMs
    return UInt64(max(p, 0))
}

/// Desktop `seek_target`: where to seek so a line shows at its time.
private func seekTarget(_ start: UInt64, _ m: LyricsModel) -> UInt64 {
    UInt64(max(Int64(start) - (m.globalOffsetMs + m.songOffsetMs), 0))
}

/// The lines being sung at `pos` (more than one where voices overlap) and the one
/// the view follows: the first of those, else the last that started. The core
/// worked out each line's `activeEndMs` (shared Rust `Lyrics::active`/`line_at`).
private func singing(_ lines: [LyricLine], _ pos: UInt64) -> (focus: Int?, active: [Int]) {
    var lo = 0, hi = lines.count
    while lo < hi {
        let mid = (lo + hi) / 2
        if lines[mid].startMs <= pos { lo = mid + 1 } else { hi = mid }
    }
    let active = (0..<lo).filter { pos < lines[$0].activeEndMs }
    return (active.first ?? (lo > 0 ? lo - 1 : nil), active)
}

/// How much of each word or syllable is sung at `pos`, 0 to 1 (the core worked out each end).
private func progress(_ tokens: [LyricToken], _ pos: UInt64) -> [Double] {
    tokens.map { t in
        pos <= t.startMs ? 0 : min(Double(pos - t.startMs) / Double(max(t.endMs - t.startMs, 1)), 1)
    }
}

/// Synced lyrics: the current line at the theme's anchor (top / upper / middle),
/// lines gliding (or fading, popping) into place, every line being sung lit (voices
/// can overlap), word/syllable timing swept across each word as it's sung, backing
/// vocals under their line, a duet's voices on two sides, distance fade, tap a line
/// to play from it, drag to read ahead ("↓ back to current line").
struct SyncedLyrics: View {
    @Environment(Store.self) private var store
    @Environment(\.accessibilityReduceMotion) private var systemReduce
    @ScaledMetric(relativeTo: .title3) private var unit: CGFloat = 1
    let m: LyricsModel
    let embedded: Bool
    @State private var current: Int?
    @State private var active: [Int] = []
    @State private var manual = false

    var body: some View {
        let look = Look.shared
        let style = look.lyrics
        let anchorY: CGFloat = style.position == "top" ? 0.2 : style.position == "middle" ? 0.5 : 0.38
        let reduce = systemReduce || look.reduceMotion
        let duet = m.lines.contains { $0.voice == .other }
        GeometryReader { g in
            // Dynamic Type scales the lyrics too (a larger cap for the accessibility sizes).
            let size = min(max(g.size.width * 0.05 * CGFloat(style.size) * 1.6, 15) * unit, 44 * max(unit, 1)).rounded()
            let gap = size * 0.55 * CGFloat(style.spacing)
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(spacing: gap) {
                        Color.clear.frame(height: g.size.height * anchorY)
                        ForEach(Array(m.lines.enumerated()), id: \.offset) { i, line in
                            LyricRow(m: m, index: i, line: line, size: size, focus: current, lit: active.contains(i), duet: duet, reduce: reduce)
                                .id(i)
                                .onTapGesture {
                                    store.core.seek(positionMs: seekTarget(line.startMs, m))
                                    manual = false
                                }
                                .accessibilityAddTraits(.isButton)
                                .accessibilityHint("Plays from this line")
                        }
                        Color.clear.frame(height: g.size.height * (1 - anchorY))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 4)
                }
                .scrollIndicators(.hidden)
                .onScrollPhaseChange { _, phase in
                    if phase == .interacting { manual = true }
                }
                .overlay(alignment: .bottom) {
                    if manual {
                        Button {
                            manual = false
                            if let c = current { withAnimation(reduce ? nil : .easeOut(duration: 0.4)) { proxy.scrollTo(c, anchor: UnitPoint(x: 0.5, y: anchorY)) } }
                        } label: {
                            Text("↓ back to current line").gFont(12.5).foregroundStyle(look.foreground)
                                .padding(.horizontal, 14).padding(.vertical, 8).panel()
                        }
                        .buttonStyle(.plain)
                        .padding(.bottom, 8)
                        .accessibilityLabel("Back to the current line")
                    }
                }
                .background(
                    // Which lines are sung: ten looks a second (the words themselves are drawn per frame by their rows).
                    TimelineView(.periodic(from: .now, by: 0.1)) { ctx in
                        let now = singing(m.lines, lyricsPosition(store, m, at: ctx.date))
                        Color.clear
                            .onChange(of: now.active, initial: true) { _, new in active = new }
                            .onChange(of: now.focus, initial: true) { _, new in
                                guard new != current else { return }
                                current = new
                                guard !manual, let new else { return }
                                let secs = 0.55 / max(Double(style.speed), 0.1)
                                if style.animation == "none" {
                                    proxy.scrollTo(new, anchor: UnitPoint(x: 0.5, y: anchorY))
                                } else if reduce || style.animation == "fade" {
                                    withAnimation(.easeInOut(duration: secs * 0.6)) { proxy.scrollTo(new, anchor: UnitPoint(x: 0.5, y: anchorY)) }
                                } else if style.animation == "pop" {
                                    withAnimation(.spring(response: secs, dampingFraction: 0.62)) { proxy.scrollTo(new, anchor: UnitPoint(x: 0.5, y: anchorY)) }
                                } else {
                                    withAnimation(.easeOut(duration: secs)) { proxy.scrollTo(new, anchor: UnitPoint(x: 0.5, y: anchorY)) }
                                }
                            }
                    }
                )
            }
        }
        .frame(minHeight: embedded ? 420 : 200)
    }
}

/// One line: lit while it's sung, faded with distance otherwise; word or syllable
/// timing swept across each word (only when the provider timed them); backing vocals
/// under it; the translation under it ("below") or in its place ("only").
struct LyricRow: View {
    @Environment(Store.self) private var store
    let m: LyricsModel
    let index: Int
    let line: LyricLine
    let size: CGFloat
    let focus: Int?
    /// Being sung now.
    let lit: Bool
    let duet: Bool
    let reduce: Bool

    var body: some View {
        let look = Look.shared
        let style = look.lyrics
        let dist = Double(abs((focus ?? -10) - index))
        let fade = 1 - min(dist / 6, 1) * Double(style.fade)
        let currentColor = look.lyricsCurrent
        let otherColor = look.lyricsOther.opacity(max(fade, 0.18))
        let translated = translation
        let only = m.translationMode == .only && translated != nil
        let text = only ? translated! : line.text
        VStack(alignment: hAlign, spacing: size * 0.14) {
            if text.isEmpty {
                Text("♪").font(.system(size: size * 0.8, design: .monospaced)).foregroundStyle(lit ? currentColor : otherColor)
            } else if lit, !only, !line.tokens.isEmpty {
                SungText(m: m, tokens: line.tokens, font: font(bold: style.bold), base: currentColor, reduce: reduce)
            } else {
                Text(text)
                    .font(font(bold: lit && style.bold))
                    .foregroundStyle(lit ? currentColor : otherColor)
            }
            if let b = line.backing, !only {
                let small = Font.system(size: (size * 0.66).rounded(), weight: .regular, design: .monospaced)
                if lit, !b.tokens.isEmpty {
                    SungText(m: m, tokens: b.tokens, font: small, base: currentColor.opacity(0.8), reduce: reduce)
                } else {
                    Text(b.text).font(small).foregroundStyle(lit ? currentColor.opacity(0.75) : otherColor.opacity(0.75))
                }
            }
            if m.translationMode == .below, let t = translated, !t.isEmpty, t.lowercased() != line.text.lowercased() {
                Text(t).font(.system(size: (size * 0.58).rounded(), design: .monospaced)).foregroundStyle(look.muted.opacity(lit ? 1 : fade))
            }
        }
        .multilineTextAlignment(textAlign)
        .frame(maxWidth: .infinity, alignment: frameAlign)
        // Right-to-left lines read (and align, and sweep) from the right.
        .environment(\.layoutDirection, line.rtl ? .rightToLeft : .leftToRight)
        .scaleEffect(lit || reduce ? 1 : 0.98, anchor: anchor)
        .animation(reduce ? nil : .easeOut(duration: 0.8 * 0.55 / max(Double(style.speed), 0.1)), value: lit)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityText(text))
        .accessibilityAddTraits(lit ? .isSelected : [])
    }

    private func accessibilityText(_ text: String) -> String {
        if text.isEmpty { return "instrumental" }
        if let b = line.backing, !b.text.isEmpty { return "\(text). Backing vocals: \(b.text)" }
        return text
    }

    private var translation: String? {
        guard let t = m.translation, index < t.lines.count else { return nil }
        let s = t.lines[index]
        return s.isEmpty ? nil : s
    }

    private func font(bold: Bool) -> Font { .system(size: size, weight: bold ? .bold : .regular, design: .monospaced) }

    /// The theme's alignment in reading order; a duet's other voice on the other side
    /// (centered lyrics put a duet's lead at the start and the other voice at the end).
    private var side: String {
        let a = Look.shared.lyrics.align
        switch line.voice {
        case .other: return a == "right" ? "left" : "right"
        case .lead where duet && a == "center": return "left"
        default: return a
        }
    }
    private var hAlign: HorizontalAlignment { side == "left" ? .leading : side == "right" ? .trailing : .center }
    private var textAlign: TextAlignment { side == "left" ? .leading : side == "right" ? .trailing : .center }
    private var frameAlign: Alignment { side == "left" ? .leading : side == "right" ? .trailing : .center }
    private var anchor: UnitPoint { side == "left" ? .leading : side == "right" ? .trailing : .center }
}

/// Words or syllables sung across their length: each sweeps from waiting to lit
/// as it's sung (with Reduce Motion, brightens in place instead), drawn every
/// frame while the song plays. Only drawing changes per frame: the text and its
/// layout stay the same.
struct SungText: View {
    @Environment(Store.self) private var store
    let m: LyricsModel
    let tokens: [LyricToken]
    let font: Font
    /// The line's lit color.
    let base: Color
    let reduce: Bool

    var body: some View {
        let look = Look.shared
        let text = Self.text(tokens).font(font).foregroundStyle(.white)
        // Words still to come are dim, the word being sung lights up bright, and sung
        // words settle into the line's color a moment after they're sung.
        let bright = sungColor(highlight: look.lyricsHighlight, line: base, text: look.foreground)
        TimelineView(.animation(minimumInterval: reduce ? 1.0 / 20 : nil, paused: !store.isPlaying && store.scrub == nil)) { ctx in
            let pos = lyricsPosition(store, m, at: ctx.date)
            let lit = tokens.map { t -> Color in
                let settle = pos > t.endMs ? min(Double(pos - t.endMs) / 400, 1) : 0
                return bright.mix(with: base, by: settle)
            }
            text.textRenderer(Sweep(progress: progress(tokens, pos), lit: lit, waiting: base.opacity(0.4), soft: reduce ? 0 : 0.45))
        }
    }

    /// The pieces as one text, each tagged with its index for `Sweep`.
    static func text(_ tokens: [LyricToken]) -> Text {
        var out = Text("")
        for (k, t) in tokens.enumerated() {
            let piece = Text(t.text).customAttribute(Piece(index: k))
            out = Text("\(out)\(piece)")
        }
        return out
    }
}

/// How bright a color looks (0 dark – 1 light).
private func luminance(_ c: Color) -> Float {
    let r = c.resolve(in: EnvironmentValues())
    return 0.2126 * r.linearRed + 0.7152 * r.linearGreen + 0.0722 * r.linearBlue
}

/// The color of the word being sung: the theme's highlight when it stands out from
/// the line's own color; otherwise (a dark highlight, or album colors close to the
/// line's) the line's color lifted most of the way toward the text color, so the
/// word being sung always shows (shared with the desktop's `sung_color`).
func sungColor(highlight: Color, line: Color, text: Color) -> Color {
    let (h, c, t) = (luminance(highlight), luminance(line), luminance(text))
    return (h - c) * (t >= c ? 1 : -1) > 0.12 ? highlight : line.mix(with: text, by: 0.7)
}

/// Which word or syllable a run of text is.
struct Piece: TextAttribute {
    let index: Int
}

/// Draws each piece in the waiting color with its sung share in its lit color,
/// over a soft edge (`soft` × the line height wide) that runs the way the text
/// reads; with `soft` 0 the piece brightens in place instead.
struct Sweep: TextRenderer {
    let progress: [Double]
    let lit: [Color]
    let waiting: Color
    let soft: CGFloat

    func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
        for line in layout {
            for run in line {
                guard let k = run[Piece.self]?.index, k < progress.count else {
                    ctx.draw(run)
                    continue
                }
                let p = progress[k]
                var under = ctx
                under.addFilter(.colorMultiply(waiting))
                if p >= 1 {
                    var over = ctx
                    over.addFilter(.colorMultiply(lit[k]))
                    over.draw(run)
                    continue
                }
                under.draw(run)
                guard p > 0 else { continue }
                var over = ctx
                over.addFilter(.colorMultiply(lit[k]))
                let r = run.typographicBounds.rect
                if soft <= 0 {
                    // Lit the moment it starts, fully by its end.
                    over.opacity = 0.6 + p * 0.4
                    over.draw(run)
                    continue
                }
                let band = min(r.height * soft, r.width * 0.6)
                let ltr = run.layoutDirection == .leftToRight
                // The edge runs from half a band before the piece to half a band after it.
                let edge = ltr ? r.minX - band / 2 + p * (r.width + band) : r.maxX + band / 2 - p * (r.width + band)
                over.clipToLayer { mask in
                    let start = CGPoint(x: edge - (ltr ? band / 2 : -band / 2), y: r.midY)
                    let end = CGPoint(x: edge + (ltr ? band / 2 : -band / 2), y: r.midY)
                    mask.fill(Path(r.insetBy(dx: -band, dy: -r.height)), with: .linearGradient(Gradient(colors: [.white, .clear]), startPoint: start, endPoint: end))
                }
                over.draw(run)
            }
        }
    }
}

/// Plain (unsynced) lyrics: static, in the current color, with stanza gaps.
struct PlainLyrics: View {
    let m: LyricsModel
    let embedded: Bool
    var body: some View {
        let look = Look.shared
        let color = look.lyricsCurrent
        let content = VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(m.lines.enumerated()), id: \.offset) { i, l in
                let t = m.translation.flatMap { i < $0.lines.count ? $0.lines[i] : nil } ?? ""
                if l.text.isEmpty {
                    Color.clear.frame(height: 12)
                } else if m.translationMode == .only, !t.isEmpty {
                    Text(t).gFont(16).foregroundStyle(color)
                } else {
                    Text(l.text).gFont(16).foregroundStyle(color)
                    if m.translationMode == .below, !t.isEmpty, t.lowercased() != l.text.lowercased() {
                        Text(t).gFont(12.5).foregroundStyle(look.muted)
                    }
                }
            }
        }
        .textSelection(.enabled)
        if embedded { content } else { ScrollView { content }.scrollIndicators(.hidden) }
    }
}

/// "translating…", "from Japanese · Apple, on this device", "already in English", or why not.
struct TranslationNote: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let t: TranslationModel
    @State private var download: TranslationSession.Configuration?

    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if t.working { ProgressView().controlSize(.mini).tint(look.sub) }
                Text(t.note).gFont(11.5).foregroundStyle(t.fix == nil ? look.muted : look.sub).fixedSize(horizontal: false, vertical: true)
            }
            if t.fix == .downloadLanguage, let pair = AppleTranslator.lastNeedsDownload {
                BracketButton(title: "download \(AppleTranslator.name(pair.source)) → \(AppleTranslator.name(pair.target))", icon: "arrow.down.circle", primary: true) { startDownload(pair) }
                Text("A one-time, free download; then this iPhone translates by itself.").gFont(11).foregroundStyle(look.muted).fixedSize(horizontal: false, vertical: true)
            } else if t.fix == .turnOnAssistant {
                Button("assistant settings") { store.nowPlayingOpen = false; store.tab = .settings; store.settingsQuery = "assistant" }
                    .buttonStyle(.plain).gFont(11.5).foregroundStyle(accent)
            } else if t.failed {
                Button("try again") { store.core.retryTranslation() }.buttonStyle(.plain).gFont(11.5).foregroundStyle(accent)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Turning translation on (文A) when the languages aren't downloaded yet asks for
        // the download straight away, instead of leaving it to this button.
        .onChange(of: t.fix, initial: true) { _, fix in
            if fix == .downloadLanguage, AppleTranslator.justAsked, let pair = AppleTranslator.lastNeedsDownload {
                AppleTranslator.askedAt = nil
                startDownload(pair)
            }
        }
        // The system's own sheet downloads the languages; then translate again.
        .translationTask(download) { session in
            do { try await session.prepareTranslation() } catch {}
            await MainActor.run {
                download = nil
                store.core.retryTranslation()
            }
        }
    }

    private func startDownload(_ pair: (source: String, target: String)) {
        download = TranslationSession.Configuration(source: Locale.Language(identifier: pair.source), target: Locale.Language(identifier: pair.target))
    }
}

/// 文A mode, "wrong lyrics?", this song's timing (− / +, tap to reset), and where they came from.
/// One row when it fits, two when it doesn't (it used to push Now Playing off the screen).
struct LyricsFooter: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let m: LyricsModel
    @State private var showDetail = false
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 4) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 6) {
                    modeButton
                    Spacer(minLength: 4)
                    wrongLyrics
                    timing
                }
                VStack(alignment: .leading, spacing: 0) {
                    HStack(spacing: 6) {
                        modeButton
                        Spacer(minLength: 4)
                        wrongLyrics
                    }
                    HStack(spacing: 6) {
                        Spacer(minLength: 0)
                        timing
                    }
                }
            }
            Button { showDetail.toggle() } label: {
                Text(m.pending ? m.provenance + " · looking for word timing…" : m.provenance).gFont(11).foregroundStyle(look.muted).lineLimit(1)
            }
            .buttonStyle(.plain)
            if showDetail { Text(m.detail).gFont(11).foregroundStyle(look.muted).textSelection(.enabled) }
        }
    }

    private var modeButton: some View {
        let look = Look.shared
        return Button {
            let next: String = switch m.translationMode { case .off: "below"; case .below: "only"; case .only: "off" }
            if m.translationMode == .off { AppleTranslator.askedAt = Date() }
            store.core.setLyricsTranslation(mode: next)
            store.show(toast: "Lyrics: " + (next == "off" ? "original" : next == "below" ? "original + translation" : "translated"))
        } label: {
            Text("文A " + (m.translationMode == .off ? "original" : m.translationMode == .below ? "+ translation" : "translated"))
                .gFont(12).lineLimit(1).fixedSize().foregroundStyle(m.translationMode == .off ? look.sub : accent).frame(minHeight: 44)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Translation: " + (m.translationMode == .off ? "original" : m.translationMode == .below ? "original and translation" : "translated"))
    }

    private var wrongLyrics: some View {
        Button { store.core.lyricsOpenPicker() } label: { Text("wrong lyrics?").gFont(12).lineLimit(1).fixedSize().foregroundStyle(Look.shared.sub).frame(minHeight: 44) }.buttonStyle(.plain)
    }

    private var timing: some View {
        let look = Look.shared
        return HStack(spacing: 0) {
            Button { store.core.setSongLyricsOffset(ms: m.songOffsetMs - 100) } label: { Text("−").gFont(15).foregroundStyle(look.sub).frame(width: 32, height: 44) }
                .buttonStyle(.plain).accessibilityLabel("Show lyrics 0.1 s later for this song")
            Button { store.core.setSongLyricsOffset(ms: 0) } label: {
                Text(m.songOffsetMs == 0 ? "timing 0.0 s" : String(format: "timing %+.1f s", Double(m.songOffsetMs) / 1000)).gFont(12).lineLimit(1).fixedSize().foregroundStyle(look.sub)
            }
            .buttonStyle(.plain).accessibilityHint("Resets this song's timing")
            Button { store.core.setSongLyricsOffset(ms: m.songOffsetMs + 100) } label: { Text("+").gFont(15).foregroundStyle(look.sub).frame(width: 32, height: 44) }
                .buttonStyle(.plain).accessibilityLabel("Show lyrics 0.1 s earlier for this song")
        }
    }
}

/// "choose lyrics for this song": search LRCLIB, pick one, or none of these.
struct PickerView: View {
    @Environment(Store.self) private var store
    let m: LyricsModel
    let picker: PickerModel
    @State private var query = ""
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 8) {
            Text("choose lyrics for this song").gFont(17).foregroundStyle(look.foreground)
            Text("\(m.title) — \(m.artists) · \(Fmt.ms(m.durationMs))").gFont(12.5).foregroundStyle(look.sub)
            FilterField(text: $query, prompt: "search LRCLIB")
                .onSubmit { store.core.lyricsSearch(query: query) }
            if picker.loading {
                LoadingNote()
            } else if let e = picker.error {
                Text("search failed").gFont(14).foregroundStyle(look.foreground)
                Text(e).gFont(12).foregroundStyle(look.sub)
            } else if picker.results.isEmpty {
                Unavailable(title: "nothing found", detail: "Try fewer words, or just the title.")
            } else {
                CandidateList(candidates: picker.results)
            }
            HStack {
                BracketButton(title: "none of these") { store.core.lyricsNoneFit() }
                BracketButton(title: "cancel") { store.core.lyricsClosePicker() }
            }
        }
        .onAppear { query = picker.query }
    }
}

struct CandidateList: View {
    @Environment(Store.self) private var store
    let candidates: [CandidateModel]
    var body: some View {
        let look = Look.shared
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(candidates.enumerated()), id: \.offset) { i, c in
                Button { store.core.lyricsPick(index: UInt32(i)) } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(c.title).gFont(13.5).foregroundStyle(look.foreground).lineLimit(1)
                        Text(c.meta).gFont(11.5).foregroundStyle(look.sub).lineLimit(1)
                        if !c.preview.isEmpty { Text("“\(c.preview)”").gFont(11.5).foregroundStyle(look.muted).lineLimit(1) }
                    }
                    .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                Hairline()
            }
        }
    }
}
