import SwiftUI

/// Search (desktop pages.rs `search`): the field ("search Spotify — songs, artists,
/// albums, podcasts", 350 ms after typing stops or at once on return), recent
/// searches and Browse all when it's empty, then the filter chips and results.
struct SearchScreen: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    @State private var committed = ""
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var store = store
        let look = Look.shared
        let route = Route.search(query: committed, filter: store.searchFilter)
        ScrollView {
            LazyVStack(alignment: .leading, spacing: look.sp(1.5)) {
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").foregroundStyle(look.sub)
                    Text(">").gFont(14).foregroundStyle(focused ? accent : look.sub)
                    TextField("", text: $store.searchText, prompt: Text("search Spotify — songs, artists, albums, podcasts").foregroundColor(look.muted))
                        .gFont(14).foregroundStyle(look.foreground)
                        .focused($focused)
                        .submitLabel(.search)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                        .onSubmit { commit(now: true) }
                    if !store.searchText.isEmpty {
                        Button { store.searchText = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(look.muted) }.buttonStyle(.plain).accessibilityLabel("Clear search")
                    }
                }
                .padding(.horizontal, 14).frame(height: 48)
                .background(RoundedRectangle(cornerRadius: look.smallRadius).fill(focused ? look.raised : look.surface))
                .overlay(RoundedRectangle(cornerRadius: look.smallRadius).strokeBorder(focused ? accent : look.divider, lineWidth: focused ? 1.2 : 1))
                if !committed.isEmpty {
                    ScrollView(.horizontal) {
                        HStack(spacing: 8) {
                            ForEach(filters, id: \.1) { f, name in
                                Chip(title: name, on: store.searchFilter == f) { store.searchFilter = f }
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                }
                SearchResults(route: route, query: committed)
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.interactively)
        .glassNav("search")
        .task(id: store.searchText) {
            try? await Task.sleep(for: .milliseconds(350))
            if !Task.isCancelled { commit(now: false) }
        }
        .simultaneousGesture(TapGesture().onEnded { remember() })
    }

    private var filters: [(SearchFilter, String)] {
        [(.all, "All"), (.songs, "Songs"), (.artists, "Artists"), (.albums, "Albums"), (.playlists, "Playlists"), (.podcasts, "Podcasts & Shows"), (.episodes, "Episodes"), (.audiobooks, "Audiobooks")]
    }

    private func commit(now: Bool) {
        let q = store.searchText.trimmingCharacters(in: .whitespaces)
        committed = q
        if now { remember() }
    }

    /// Any tap on the results keeps the query in recent searches (desktop `remember_search`).
    private func remember() {
        if !committed.isEmpty { store.core.rememberSearch(query: committed) }
    }
}

struct SearchResults: View {
    @Environment(Store.self) private var store
    let route: Route
    let query: String
    var body: some View {
        let page = store.page(route)
        Group {
            switch page?.state {
            case .none, .loading?:
                if let page { BlocksView(page: page) }
                LoadingNote()
            case let .failed(message)?:
                ErrorBox(message: message) { store.core.refresh(key: "/search") }
            case let .missing(title, detail)?:
                Unavailable(title: title, detail: detail)
            case .ready?:
                if let page { BlocksView(page: page) }
            }
        }
        .onAppear { store.subscribe(route) }
        .onDisappear { store.unsubscribe(route) }
        .onChange(of: route) { old, new in
            store.unsubscribe(old)
            store.subscribe(new)
        }
    }
}
