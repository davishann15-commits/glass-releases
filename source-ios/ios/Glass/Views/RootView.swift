import SwiftUI

/// The shell: sign-in, or Glass's destinations (Home, Library, Search, Local Files,
/// Settings) with the persistent player bar. Compact widths get a bottom bar;
/// regular widths (iPad, large landscape) the desktop's side rail.
struct RootView: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @Environment(\.horizontalSizeClass) private var hSize

    var body: some View {
        @Bindable var store = store
        ZStack {
            // Album colors everywhere: the wash is behind every page, blooming from the
            // player bar's cover when the song changes.
            GlassBackground(wash: Look.shared.albumScope == .everywhere, bloomFrom: UnitPoint(x: 0.1, y: 0.92))
            if store.account.phase == .signedIn || store.demo {
                Group {
                    if hSize == .regular { wide } else { compact }
                }
                .accessibilityHidden(store.nowPlayingOpen)
            } else {
                SignInView()
            }
            // Now Playing slides up over the app (and can be pulled back down, with the
            // app showing behind it as it goes).
            if store.nowPlayingOpen {
                NowPlayingView()
                    .transition(.move(edge: .bottom))
                    .zIndex(10)
                    .accessibilityAddTraits(.isModal)
            }
            ToastView().zIndex(11)
        }
        .animation(.spring(response: 0.44, dampingFraction: 0.9), value: store.nowPlayingOpen)
        .tint(accent)
        .sheet(isPresented: $store.themePanel) {
            ThemePanel().environment(store)
        }
        .sheet(item: $store.sheet) { kind in
            SheetHost(kind: kind).environment(store)
        }
        .alert(store.confirm?.title ?? "", isPresented: Binding(get: { store.confirm != nil }, set: { if !$0 { store.confirm = nil } }), presenting: store.confirm) { c in
            Button(c.button, role: .destructive) { c.run() }
            Button("Cancel", role: .cancel) {}
        } message: { c in
            Text(c.body)
        }
        .background(SignInSession())
        .background(AssistantSignInSession())
        // The assistant: the desktop's right panel — a column on iPad, a sheet on iPhone.
        .inspector(isPresented: $store.assistantOpen) {
            AssistantPanel().environment(store)
                .inspectorColumnWidth(min: 320, ideal: 400, max: 480)
                .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $store.commandOpen) { CommandSheet().environment(store) }
        .background { KeyCommands() }
        // ⌘J opens and closes it (desktop ⌘J), with a keyboard attached.
        .background {
            Button("Assistant") { if store.assistant?.enabled == true || store.assistantOpen { store.assistantOpen.toggle() } }
                .keyboardShortcut("j", modifiers: .command)
                .hidden()
                .accessibilityHidden(true)
        }
        .onChange(of: screenKey, initial: true) { store.core.setScreen(route: store.topRoute, assistant: store.assistantOpen) }
        // The one-time notice after the first sign-in that enables plan use (desktop modals.rs AiWelcome).
        .alert("You're using your ChatGPT plan", isPresented: Binding(get: { store.assistant?.welcome == true }, set: { if !$0 { store.core.aiDismissWelcome() } })) {
            Button("Got it") { store.core.aiDismissWelcome() }
            Button("Manage usage") { openManageUsage(); store.core.aiDismissWelcome() }
        } message: {
            Text("Eligible usage in this app uses your ChatGPT plan. Manage usage in your ChatGPT settings.")
        }
        .fileImporter(isPresented: Binding(get: { store.addFilesTarget != nil }, set: { if !$0 { store.addFilesTarget = nil } }), allowedContentTypes: LocalFilesScreen.audioTypes, allowsMultipleSelection: true) { r in
            let target = store.addFilesTarget ?? .localFiles
            store.addFilesTarget = nil
            if case let .success(urls) = r { LocalImport.importFiles(urls, store: store, target: target) }
        }
        // Audio files dragged in from Files or another app (iPad): to the playlist on screen, else Local Files.
        .onDrop(of: [.audio], isTargeted: nil) { providers in
            LocalImport.drop(providers, store: store)
        }
    }

    private var compact: some View {
        VStack(spacing: 0) {
            Tabs()
            PlayerBar()
                .padding(.horizontal, 8)
                .padding(.bottom, 4)
            NavBar()
        }
    }

    /// What the core is told about the screen: the page on top and the assistant.
    private var screenKey: String {
        "\(store.tab)|\(String(describing: store.topRoute))|\(store.assistantOpen)"
    }

    private var wide: some View {
        HStack(spacing: 0) {
            Rail()
            Rectangle().fill(Look.shared.divider).frame(width: 1).ignoresSafeArea()
            VStack(spacing: 0) {
                Tabs()
                PlayerBar(wide: true)
                    .padding(.horizontal, 12)
                    .padding(.bottom, 8)
            }
        }
    }
}

/// The five destinations, each keeping its own stack and place. The system's tab view
/// does the keeping (and takes the ones not shown out of VoiceOver's way); its own tab
/// bar is hidden, Glass's bar or rail switches.
struct Tabs: View {
    @Environment(Store.self) private var store
    var body: some View {
        TabView(selection: Binding(get: { store.tab }, set: { store.tab = $0 })) {
            ForEach(Tab.allCases, id: \.self) { t in
                TabStack(tab: t)
                    .toolbarVisibility(.hidden, for: .tabBar)
                    .tag(t)
            }
        }
        .tabViewStyle(.tabBarOnly)
    }
}

/// One destination's navigation stack.
struct TabStack: View {
    @Environment(Store.self) private var store
    let tab: Tab

    var body: some View {
        NavigationStack(path: store.path(tab)) {
            root
                // Each page sits on Glass's own glass (the system would paint plain black),
                // with the album's colors when they apply everywhere.
                .containerBackground(for: .navigation) { PageBackground() }
                .navigationDestination(for: Route.self) { r in
                    destination(r).containerBackground(for: .navigation) { PageBackground() }
                }
        }
    }

    @ViewBuilder private func destination(_ r: Route) -> some View {
        switch r {
        case let .localAlbum(artist, album): LocalAlbumScreen(artist: artist, album: album)
        case .localFiles: LocalFilesScreen()
        case .settings: SettingsScreen()
        case .activity: ActivityScreen()
        case let .device(id, name): DeviceLibraryScreen(peer: id, name: name)
        case .library: LibraryScreen()
        case .search: SearchScreen()
        default: PageScreen(route: r)
        }
    }

    @ViewBuilder private var root: some View {
        switch tab {
        case .home: PageScreen(route: .home)
        case .library: LibraryScreen()
        case .search: SearchScreen()
        case .local: LocalFilesScreen()
        case .settings: SettingsScreen()
        }
    }
}

/// The bottom bar: five destinations, lowercase and quiet.
struct NavBar: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        HStack(spacing: 0) {
            ForEach(Tab.allCases, id: \.self) { t in
                Button {
                    if store.tab == t { store.paths[t] = [] } else { store.tab = t }
                } label: {
                    VStack(spacing: 3) {
                        Image(systemName: t.icon).font(.system(size: 17, weight: store.tab == t ? .semibold : .regular))
                        Text(t.title).gFont(10)
                    }
                    .foregroundStyle(store.tab == t ? accent : look.muted)
                    .frame(maxWidth: .infinity, minHeight: 48)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(t.title)
                .accessibilityAddTraits(store.tab == t ? .isSelected : [])
            }
        }
        .padding(.top, 2)
        // The same glass as the pages (the theme, and album colors where they apply),
        // which the app's background already is under the bar.
        .overlay(alignment: .top) { Hairline() }
    }
}

/// The desktop's left rail, for wide layouts.
struct Rail: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        VStack(spacing: 6) {
            Text("glass").gFont(12).foregroundStyle(look.muted).padding(.vertical, 12)
            ForEach(Tab.allCases, id: \.self) { t in
                Button {
                    if store.tab == t { store.paths[t] = [] } else { store.tab = t }
                } label: {
                    VStack(spacing: 4) {
                        Image(systemName: t.icon).font(.system(size: 19))
                        Text(t.title).gFont(10)
                    }
                    .foregroundStyle(store.tab == t ? accent : look.muted)
                    .frame(width: 76, height: 60)
                    .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(store.tab == t ? look.selection : .clear))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(t.title)
            }
            Spacer()
            if store.assistant?.enabled == true {
                Button { store.assistantOpen.toggle() } label: {
                    VStack(spacing: 4) {
                        Image(systemName: "sparkle").font(.system(size: 17))
                        Text("assistant").gFont(10)
                    }
                    .foregroundStyle(store.assistantOpen ? accent : look.muted)
                    .frame(width: 76, height: 56)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Assistant")
                .padding(.bottom, 12)
            }
        }
        .frame(width: 88)
    }
}

/// "> Added to queue": the newest toast, for 3 s, above the player.
struct ToastView: View {
    @Environment(Store.self) private var store
    var body: some View {
        VStack {
            Spacer()
            if let t = store.toast {
                Text("> " + t.text)
                    .gFont(13)
                    .foregroundStyle(Look.shared.foreground)
                    .padding(.horizontal, 14).padding(.vertical, 10)
                    .panel()
                    .padding(.bottom, 150)
                    .padding(.horizontal, 20)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .id(t.id)
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
        .animation(.easeOut(duration: 0.2), value: store.toast)
        .allowsHitTesting(false)
    }
}

/// A screen's title as the desktop's breadcrumb: "~ / music / playlist".
struct Breadcrumb: View {
    let path: String
    var body: some View {
        Text("~ / " + path).gFont(12).foregroundStyle(Look.shared.muted).lineLimit(1)
    }
}

extension View {
    /// Glass's navigation bar: transparent, breadcrumb title, quiet back arrow.
    func glassNav(_ path: String) -> some View {
        toolbar {
            ToolbarItem(placement: .principal) { Breadcrumb(path: path) }
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 2) {
                    CommandButton()
                    AssistantButton()
                }
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbarBackground(.hidden, for: .navigationBar)
    }
}

/// A page's background: the glass, washed with the album's colors when they apply
/// everywhere (blooming from the player bar's cover when the song changes).
struct PageBackground: View {
    var body: some View {
        GlassBackground(wash: Look.shared.albumScope == .everywhere, bloomFrom: UnitPoint(x: 0.1, y: 0.95))
    }
}
