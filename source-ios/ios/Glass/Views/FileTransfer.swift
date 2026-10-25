import SwiftUI
import UniformTypeIdentifiers

/// [Export] and [Import] for theme.toml and config.toml: on iOS there's no text editor
/// to open them in, so they go out through the share sheet (Files, Mail, another
/// device) and come back in from Files. An import that doesn't parse changes nothing.
struct FileTransferButtons: View {
    @Environment(Store.self) private var store
    let which: EditableFile
    @State private var exporting: URL?
    @State private var importing = false

    var body: some View {
        HStack(spacing: 0) {
            BracketButton(title: "Export") {
                Task {
                    do {
                        exporting = URL(fileURLWithPath: try await store.core.exportFile(which: which))
                    } catch {
                        store.show(toast: error.plain)
                    }
                }
            }
            BracketButton(title: "Import") { importing = true }
        }
        .sheet(item: $exporting) { url in ShareSheet(items: [url]).presentationDetents([.medium, .large]) }
        .fileImporter(isPresented: $importing, allowedContentTypes: [UTType(filenameExtension: "toml") ?? .plainText, .plainText, .text]) { r in
            guard case let .success(url) = r else { return }
            let ok = url.startAccessingSecurityScopedResource()
            defer { if ok { url.stopAccessingSecurityScopedResource() } }
            guard let text = try? String(contentsOf: url, encoding: .utf8) else {
                store.show(toast: "That file isn't text Glass can read")
                return
            }
            Task {
                do {
                    store.show(toast: try await store.core.importFile(which: which, text: text))
                } catch {
                    store.show(toast: error.plain)
                }
            }
        }
    }
}

extension Error {
    /// What to tell the user: the core's own words (its errors are already plain),
    /// never Swift's description of the error value.
    var plain: String {
        if case let .Failed(message) = self as? CoreError { return message }
        return "That didn't work. Try again."
    }
}

extension URL: @retroactive Identifiable {
    public var id: String { absoluteString }
}

/// The system share sheet.
struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}
