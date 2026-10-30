import SwiftUI

/// The persistent player (the desktop's floating bar). Compact: cover, title and
/// artist (or what the DJ is saying), heart, play/pause and next, with a thin
/// progress line. Tap it for Now Playing. Wide layouts add the desktop's transport,
/// seek rail, devices, queue, lyrics and volume.
struct PlayerBar: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    var wide = false

    var body: some View {
        let look = Look.shared
        let p = store.playback
        let item = p?.item
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                // Tap or swipe up: Now Playing. Swipe left or right: next or previous song.
                // Hold: the song's menu.
                HStack(spacing: 10) {
                    Art(url: item?.imageSmall, size: wide ? 56 : 44)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item?.title ?? "nothing playing").gFont(14).foregroundStyle(look.foreground).lineLimit(1)
                        if let dj = p?.djCaption {
                            Text("DJ X · " + dj).gFont(12).foregroundStyle(accent).lineLimit(1)
                        } else if let item {
                            Text(item.kind == .episode ? (item.show?.name ?? "") : Fmt.artists(item.artists)).gFont(12).foregroundStyle(look.sub).lineLimit(1)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
                .onTapGesture { store.nowPlayingOpen = true }
                .modifier(HorizontalSwipe(left: item == nil ? nil : { store.core.next() }, right: item == nil ? nil : { store.core.previous() }))
                .simultaneousGesture(DragGesture(minimumDistance: 14).onEnded { v in
                    if v.translation.height < -30, abs(v.translation.height) > abs(v.translation.width) * 1.2 { store.nowPlayingOpen = true }
                })
                .contextMenu { if let item { TrackMenu(row: rowFor(item)) } }
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
                .accessibilityIdentifier("player-bar")
                .accessibilityLabel(item.map { "Now playing: \($0.title)" } ?? "Nothing playing")
                .accessibilityHint("Opens Now Playing")
                .accessibilityAction { store.nowPlayingOpen = true }
                .accessibilityAction(named: "Next song") { store.core.next() }
                .accessibilityAction(named: "Previous song") { store.core.previous() }
                if wide { WideControls() }
                if let item, item.kind == .track || item.kind == .episode {
                    HeartButton(uri: item.uri, episode: item.kind == .episode)
                }
                if !wide {
                    Button { store.core.togglePlay() } label: {
                        Image(systemName: store.isPlaying ? "pause.fill" : "play.fill").font(.system(size: 20)).foregroundStyle(look.foreground).frame(width: 44, height: 44)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(store.isPlaying ? "Pause" : "Play")
                    Button { store.core.next() } label: {
                        Image(systemName: "forward.fill").font(.system(size: 16)).foregroundStyle(look.sub).frame(width: 40, height: 44)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Next")
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 6)
            if !wide { ProgressLine() }
            if let hint = barHint(p) {
                Text(hint).gFont(11).foregroundStyle(look.muted).lineLimit(1).padding(.bottom, 4)
            }
        }
        .panel()
        .albumTinted()
    }

    private func barHint(_ p: PlaybackModel?) -> String? {
        guard let p else { return nil }
        if !p.premium { return "playing needs Spotify Premium" }
        if let d = p.device, !p.here { return "playing on \(d.name)" }
        return nil
    }
}

/// The thin progress line under the compact bar.
struct ProgressLine: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    var body: some View {
        TimelineView(.periodic(from: .now, by: store.isPlaying ? 0.5 : 60)) { ctx in
            GeometryReader { g in
                let d = Double(store.playback?.item?.durationMs ?? 0)
                let f = d > 0 ? Double(store.position(at: ctx.date)) / d : 0
                ZStack(alignment: .leading) {
                    Rectangle().fill(Look.shared.track)
                    Rectangle().fill(accent).frame(width: g.size.width * f)
                }
            }
            .frame(height: 2)
            .padding(.horizontal, 10)
            .padding(.bottom, 4)
        }
        .accessibilityHidden(true)
    }
}

/// Transport, seek and side buttons for the wide bar (desktop nowbar center/right).
struct WideControls: View {
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        let p = store.playback
        let episode = p?.item?.kind == .episode
        VStack(spacing: 2) {
            HStack(spacing: 6) {
                if episode {
                    IconButton(icon: "gobackward.15", size: 16, label: "Back 15 seconds") { store.core.seekBy(deltaMs: -15000) }
                } else {
                    IconButton(icon: "shuffle", size: 15, on: p?.shuffle ?? false, label: "Shuffle") { store.core.setShuffle(on: !(p?.shuffle ?? false)) }
                }
                IconButton(icon: "backward.fill", size: 15, label: "Previous") { store.core.previous() }
                PlayCircle(playing: store.isPlaying, size: 40) { store.core.togglePlay() }
                IconButton(icon: "forward.fill", size: 15, label: "Next") { store.core.next() }
                if episode {
                    IconButton(icon: "goforward.15", size: 16, label: "Forward 15 seconds") { store.core.seekBy(deltaMs: 15000) }
                } else {
                    IconButton(icon: p?.repeat == .track ? "repeat.1" : "repeat", size: 15, on: p?.repeat != .off && p != nil, label: "Repeat") { store.core.cycleRepeat() }
                }
            }
            SeekRail().frame(maxWidth: 420)
        }
        .frame(maxWidth: .infinity)
        HStack(spacing: 0) {
            IconButton(icon: "hifispeaker.2", size: 15, on: !(p?.here ?? true), label: "Devices") { store.nowPlayingView = .devices; store.nowPlayingOpen = true }
            IconButton(icon: "list.bullet", size: 15, label: "Queue") { store.nowPlayingView = .queue; store.nowPlayingOpen = true }
            IconButton(icon: "quote.bubble", size: 15, label: "Lyrics") { store.nowPlayingView = .lyrics; store.nowPlayingOpen = true }
            VolumeRail().frame(width: 110)
        }
        .foregroundStyle(look.sub)
    }
}

/// Elapsed · rail · total; drag to seek (sent on release, as the desktop does).
struct SeekRail: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @State private var drag: Double?

    var body: some View {
        let look = Look.shared
        TimelineView(.periodic(from: .now, by: store.isPlaying ? 0.5 : 60)) { ctx in
            let dur = Double(store.playback?.item?.durationMs ?? 0)
            let pos = drag.map { $0 * dur } ?? Double(store.position(at: ctx.date))
            HStack(spacing: 10) {
                Text(Fmt.ms(UInt64(pos))).gFont(11).foregroundStyle(look.sub).monospacedDigit()
                GeometryReader { g in
                    let f = dur > 0 ? min(max(pos / dur, 0), 1) : 0
                    ZStack(alignment: .leading) {
                        Capsule().fill(look.track).frame(height: 3)
                        Capsule().fill(accent).frame(width: g.size.width * f, height: 3)
                        Circle().fill(look.foreground).frame(width: drag == nil ? 9 : 14).offset(x: g.size.width * f - (drag == nil ? 4.5 : 7))
                    }
                    .frame(maxHeight: .infinity)
                    .contentShape(Rectangle())
                    .gesture(DragGesture(minimumDistance: 0)
                        .onChanged { v in
                            let f = min(max(v.location.x / g.size.width, 0), 1)
                            drag = f
                            // The lyrics follow the bar while it's held.
                            store.scrub = Store.Scrub(ms: UInt64(f * dur), since: .now, held: true)
                        }
                        .onEnded { v in
                            let f = min(max(v.location.x / g.size.width, 0), 1)
                            store.core.seek(positionMs: UInt64(f * dur))
                            drag = nil
                            // …and on from there until the player reports the new position.
                            let let_go = Store.Scrub(ms: UInt64(f * dur), since: .now, held: false)
                            store.scrub = let_go
                            Task { @MainActor in
                                try? await Task.sleep(for: .milliseconds(1500))
                                if store.scrub == let_go { store.scrub = nil }
                            }
                        })
                }
                .frame(height: 28)
                Text(Fmt.ms(UInt64(dur))).gFont(11).foregroundStyle(look.sub).monospacedDigit()
            }
        }
        .accessibilityElement()
        .accessibilityLabel("Position")
        .accessibilityValue(Fmt.ms(store.position()))
        .accessibilityAdjustableAction { dir in
            store.core.seekBy(deltaMs: dir == .increment ? 10000 : -10000)
        }
    }
}

/// Volume, with mute. Playing on this iPhone it's the iPhone's own volume (the side
/// buttons move it too); playing elsewhere, that device's.
struct VolumeRail: View {
    @Environment(Store.self) private var store
    @State private var drag: Double?

    var body: some View {
        let look = Look.shared
        let here = store.playback?.here ?? false
        let v = drag ?? Double(here ? SystemVolume.shared.level : store.playback?.volume ?? 0)
        HStack(spacing: 6) {
            Button { if here { SystemVolume.shared.toggleMute() } else { store.core.toggleMute() } } label: {
                Image(systemName: v == 0 ? "speaker.slash" : v < 0.5 ? "speaker.wave.1" : "speaker.wave.2").font(.system(size: 13)).foregroundStyle(look.sub).frame(width: 32, height: 44)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(v == 0 ? "Unmute" : "Mute")
            GeometryReader { g in
                ZStack(alignment: .leading) {
                    Capsule().fill(look.track).frame(height: 3)
                    Capsule().fill(look.foreground.opacity(0.85)).frame(width: g.size.width * v, height: 3)
                }
                .frame(maxHeight: .infinity)
                .contentShape(Rectangle())
                .gesture(DragGesture(minimumDistance: 0)
                    .onChanged { val in
                        let f = min(max(val.location.x / g.size.width, 0), 1)
                        drag = f
                        set(f, commit: false)
                    }
                    .onEnded { val in
                        set(min(max(val.location.x / g.size.width, 0), 1), commit: true)
                        drag = nil
                    })
            }
            .frame(height: 28)
        }
        .accessibilityElement()
        .accessibilityLabel("Volume")
        .accessibilityValue("\(Int((v * 100).rounded())) percent")
        .accessibilityAdjustableAction { dir in
            set(min(max(v + (dir == .increment ? 0.1 : -0.1), 0), 1), commit: true)
        }
    }

    private func set(_ f: Double, commit: Bool) {
        if store.playback?.here ?? false {
            SystemVolume.shared.set(Float(f))
        } else {
            store.core.setVolume(volume: Float(f), commit: commit)
        }
    }
}

/// The heart (songs) or + (episodes): saved state from the store, asked for when shown.
struct HeartButton: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    let uri: String
    var episode = false
    var size: CGFloat = 17

    var body: some View {
        let saved = store.isSaved(uri)
        Button { store.toggleSaved(uri) } label: {
            Image(systemName: episode ? (saved ? "checkmark.circle.fill" : "plus.circle") : (saved ? "heart.fill" : "heart"))
                .font(.system(size: size))
                .foregroundStyle(saved ? accent : Look.shared.sub)
                .frame(width: 40, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onAppear { store.wantSaved(uri) }
        .accessibilityLabel(episode ? (saved ? "Remove from Your Episodes" : "Save to Your Episodes") : (saved ? "Remove from Liked Songs" : "Save to Liked Songs"))
    }
}
