import SwiftUI
import AuthenticationServices

/// The sign-in screen (desktop mini.rs): "Glass" / "A quiet, transparent Spotify
/// player.", [ Sign in with Spotify ], and what happens.
struct SignInView: View {
    @GlassAccent private var accent
    @Environment(Store.self) private var store
    var body: some View {
        let look = Look.shared
        let waiting = store.account.phase == .signingIn
        VStack(alignment: .leading, spacing: 18) {
            Spacer()
            BlinkTitle(text: "glass")
            Text("A quiet, transparent Spotify player.").gFont(15).foregroundStyle(look.sub)
            if waiting {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 10) {
                        ProgressView().tint(accent)
                        Text("Signing in…").gFont(14).foregroundStyle(look.sub)
                    }
                    Button("Cancel") { store.core.cancelSignIn() }.buttonStyle(.plain).gFont(13).foregroundStyle(accent).frame(minHeight: 44)
                }
            } else {
                BracketButton(title: "Sign in with Spotify", primary: true) { store.core.signIn() }
            }
            if let e = store.account.error {
                Text(e).gFont(12.5).foregroundStyle(look.sub)
            }
            VStack(alignment: .leading, spacing: 10) {
                bullet("Spotify's own sign-in page opens over Glass. Glass never sees your password.")
                bullet("Glass keeps your session in this device's Keychain.")
                bullet("Playing music needs Spotify Premium.")
            }
            .padding(.top, 8)
            Spacer()
            Spacer()
        }
        .padding(28)
    }

    private func bullet(_ t: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Text("·").gFont(13).foregroundStyle(Look.shared.muted)
            Text(t).gFont(12.5).foregroundStyle(Look.shared.sub)
        }
    }
}

/// Presents Spotify's sign-in page in the system's web sign-in sheet when the core
/// asks (it's already listening for the redirect). The sheet closes when the core
/// answers Spotify's redirect with `glass-ios://spotify-signed-in`; cancelling it
/// cancels the sign-in.
struct SignInSession: View {
    @Environment(Store.self) private var store
    @Environment(\.webAuthenticationSession) private var session

    var body: some View {
        Color.clear
            .task(id: store.signInRequest?.url) {
                guard let req = store.signInRequest, let url = URL(string: req.url) else { return }
                do {
                    // Shared: an existing Spotify login in Safari carries over (one tap). The
                    // debug flag `-ephemeralSignIn` uses a private session (no system prompt).
                    let ephemeral = ProcessInfo.processInfo.arguments.contains("-ephemeralSignIn")
                    _ = try await session.authenticate(using: url, callback: .customScheme(req.callbackScheme), preferredBrowserSession: ephemeral ? .ephemeral : .shared, additionalHeaderFields: [:])
                } catch {
                    // Cancelled by the user (or the sheet failed): stop waiting.
                    if store.account.phase == .signingIn { store.core.cancelSignIn() }
                }
                store.signInRequest = nil
            }
    }
}
