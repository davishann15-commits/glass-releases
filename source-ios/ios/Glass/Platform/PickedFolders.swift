import Foundation
import Observation

/// Folders picked in Files: Local Files' sources and the save folder. iOS lets Glass
/// read one only through a security-scoped bookmark, so each is kept as one, with the
/// path it had (a folder in another app's space moves when that app is updated, and
/// songs in local playlists follow it), and opened again at each launch before the
/// core starts.
@MainActor
enum PickedFolders {
    private struct Entry: Codable {
        var bookmark: Data
        var path: String
        var source: Bool
    }

    private static let key = "pickedFolders"

    private static var entries: [Entry] {
        get { UserDefaults.standard.data(forKey: key).flatMap { try? JSONDecoder().decode([Entry].self, from: $0) } ?? [] }
        set { UserDefaults.standard.set(try? JSONEncoder().encode(newValue), forKey: key) }
    }

    /// The sources open this run.
    private(set) static var sources: [URL] = []

    /// At launch, before the core starts: each folder opened again, where it is now and
    /// where it was. One that's gone is forgotten; one that can't be opened right now
    /// is kept for the next launch.
    static func open() -> [PickedFolder] {
        migrate()
        var out: [PickedFolder] = []
        var kept: [Entry] = []
        for var e in entries {
            var stale = false
            let url: URL
            do {
                url = try URL(resolvingBookmarkData: e.bookmark, options: [], relativeTo: nil, bookmarkDataIsStale: &stale)
            } catch let error as NSError where error.domain == NSCocoaErrorDomain && [NSFileNoSuchFileError, NSFileReadNoSuchFileError].contains(error.code) {
                continue
            } catch {
                kept.append(e)
                continue
            }
            // The same folder picked twice (bookmarks kept before paths were).
            if kept.contains(where: { $0.source == e.source && same($0.path, url.path) }) { continue }
            guard url.startAccessingSecurityScopedResource() else {
                kept.append(e)
                continue
            }
            if stale, let fresh = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) { e.bookmark = fresh }
            out.append(PickedFolder(path: url.path, was: e.path.isEmpty || same(e.path, url.path) ? nil : e.path, source: e.source))
            e.path = url.path
            kept.append(e)
            if e.source { sources.append(url) }
        }
        entries = kept
        return out
    }

    /// The demo opens none, except in UI tests of adding a folder: `-demoFolders fresh`
    /// starts with none kept, `-demoFolders kept` opens the ones kept.
    static func openForDemo() -> [PickedFolder] {
        let a = ProcessInfo.processInfo.arguments
        guard let i = a.firstIndex(of: "-demoFolders") else { return [] }
        if a.indices.contains(i + 1), a[i + 1] == "fresh" { entries = [] }
        return open()
    }

    /// A folder just picked (and opened): kept for the next launches. A new save
    /// folder takes the old one's place.
    static func keep(_ url: URL, source: Bool) {
        guard let data = try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil) else { return }
        var list = entries.filter { source ? !($0.source && same($0.path, url.path)) : $0.source }
        list.append(Entry(bookmark: data, path: url.path, source: source))
        entries = list
        if source, !sources.contains(where: { same($0.path, url.path) }) { sources.append(url) }
    }

    /// A source taken out of Local Files: not opened again.
    static func forget(source path: String) {
        entries = entries.filter { !($0.source && same($0.path, path)) }
        sources.removeAll { same($0.path, path) }
    }

    /// Back to Glass's own save folder.
    static func forgetSaveFolder() {
        entries = entries.filter(\.source)
    }

    /// iOS hands out /private/var/… and /var/… for the same place.
    nonisolated static func same(_ a: String, _ b: String) -> Bool {
        plain(a) == plain(b)
    }

    nonisolated static func plain(_ p: String) -> String {
        p.hasPrefix("/private/var/") ? String(p.dropFirst("/private".count)) : p
    }

    /// Bookmarks kept by earlier builds (without their paths).
    private static func migrate() {
        let d = UserDefaults.standard
        let (sourcesKey, saveKey) = ("localFolderBookmarks", "saveFolderBookmark")
        guard d.object(forKey: sourcesKey) != nil || d.object(forKey: saveKey) != nil else { return }
        var list = entries
        for b in d.array(forKey: sourcesKey) as? [Data] ?? [] { list.append(Entry(bookmark: b, path: "", source: true)) }
        if let b = d.data(forKey: saveKey) { list.append(Entry(bookmark: b, path: "", source: false)) }
        entries = list
        d.removeObject(forKey: sourcesKey)
        d.removeObject(forKey: saveKey)
    }
}

/// Songs in a picked folder that live in iCloud Drive (or another cloud in Files) and
/// aren't on the iPhone yet: downloaded, so Local Files can play them (its scan leaves
/// them out until then), and Local Files read again as they arrive.
@MainActor
@Observable
final class CloudSongs {
    static let shared = CloudSongs()
    /// The songs still downloading.
    private(set) var waiting: Set<URL> = []
    @ObservationIgnored private var watching: Task<Void, Never>?
    @ObservationIgnored private weak var store: Store?

    func fetch(_ folders: [URL], store: Store) {
        guard !folders.isEmpty else { return }
        self.store = store
        Task.detached(priority: .utility) {
            let missing = folders.flatMap { CloudSongs.notDownloaded(in: $0) }
            for u in missing { try? FileManager.default.startDownloadingUbiquitousItem(at: u) }
            await self.wait(for: missing)
        }
    }

    private func wait(for songs: [URL]) {
        waiting.formUnion(songs)
        if watching == nil, !waiting.isEmpty { watching = Task { await watch() } }
    }

    /// Checks every 2 s; Local Files is read again at most every 8 s while songs
    /// arrive. Gives up after 5 minutes without one (offline): the next launch or
    /// rescan tries again.
    private func watch() async {
        var (read, progress, arrived) = (Date(), Date(), false)
        while !waiting.isEmpty {
            try? await Task.sleep(for: .seconds(2))
            let list = waiting
            let done = await Task.detached(priority: .utility) { list.filter(CloudSongs.downloaded) }.value
            if !done.isEmpty {
                waiting.subtract(done)
                (progress, arrived) = (Date(), true)
            }
            if arrived, waiting.isEmpty || Date().timeIntervalSince(read) > 8 {
                store?.core.rescanLocal()
                (read, arrived) = (Date(), false)
            }
            if Date().timeIntervalSince(progress) > 300 { waiting = [] }
        }
        watching = nil
    }

    /// The audio files under `folder` (as deep as Local Files reads) not on the iPhone.
    nonisolated static func notDownloaded(in folder: URL) -> [URL] {
        let fm = FileManager.default
        var out: [URL] = []
        func walk(_ dir: URL, _ depth: Int) {
            guard depth <= 5, let items = try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.isDirectoryKey, .ubiquitousItemDownloadingStatusKey]) else { return }
            for u in items {
                let name = u.lastPathComponent
                if name.hasPrefix(".") {
                    // iCloud's stand-in for a file that isn't here: ".Song.mp3.icloud".
                    if name.hasSuffix(".icloud") {
                        let real = dir.appendingPathComponent(String(name.dropFirst().dropLast(".icloud".count)))
                        if isAudio(real) { out.append(real) }
                    }
                    continue
                }
                let v = try? u.resourceValues(forKeys: [.isDirectoryKey, .ubiquitousItemDownloadingStatusKey])
                if v?.isDirectory == true {
                    walk(u, depth + 1)
                } else if isAudio(u), let s = v?.ubiquitousItemDownloadingStatus, s != .current {
                    out.append(u)
                }
            }
        }
        walk(folder, 0)
        return out
    }

    nonisolated static func downloaded(_ url: URL) -> Bool {
        var u = url
        u.removeAllCachedResourceValues()
        if let s = try? u.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]).ubiquitousItemDownloadingStatus { return s == .current }
        return FileManager.default.fileExists(atPath: u.path)
    }

    /// The kinds of file Local Files reads (src/local.rs `EXTENSIONS`).
    nonisolated static func isAudio(_ u: URL) -> Bool {
        ["mp3", "m4a", "mp4", "flac", "ogg", "oga", "wav", "aac", "aif", "aiff"].contains(u.pathExtension.lowercased())
    }
}
