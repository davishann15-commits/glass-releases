import Foundation
import NaturalLanguage
import Translation

/// Apple's on-device translation for lyrics, replacing the desktop's
/// `glass-translate` helper process with the same request and answer
/// (tools/glass-translate/main.swift): the core sends
/// `{"lines":[…],"source":"ja"|null,"target":"en"}` and reads
/// `{"status":"ok"|"same"|"needs_download"|"unsupported"|"undetermined"|"error",…}`.
/// Nothing leaves the device.
final class AppleTranslator: Translator, @unchecked Sendable {
    /// The last pair that needed a download, for the lyrics view's [download] button.
    nonisolated(unsafe) static var lastNeedsDownload: (source: String, target: String)?
    /// When 文A last turned translation on: if the languages need downloading then, the
    /// lyrics view asks for the download at once (the person just asked to translate).
    nonisolated(unsafe) static var askedAt: Date?
    static var justAsked: Bool { askedAt.map { Date().timeIntervalSince($0) < 15 } ?? false }

    /// "Japanese" for "ja", in the user's language.
    static func name(_ code: String) -> String {
        Locale.current.localizedString(forLanguageCode: base(code)) ?? code
    }

    func translate(request: String) -> String {
        guard let data = request.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let lines = obj["lines"] as? [String],
              let target = obj["target"] as? String
        else { return answer(["status": "error", "message": "bad request"]) }
        let source = obj["source"] as? String
        #if DEBUG
        // UI tests: the simulator has no on-device translation (every pair reports
        // unsupported), so `-mockTranslate` stands in, marking each line.
        if ProcessInfo.processInfo.arguments.contains("-mockTranslate") {
            return answer(["status": "ok", "source": source ?? "en", "lines": lines.map { $0.isEmpty ? "" : "[\(target)] \($0)" }])
        }
        #endif
        // The core calls this off the main thread and waits; a minute is far longer than a song takes.
        let done = DispatchSemaphore(value: 0)
        let box = Box()
        Task.detached(priority: .userInitiated) {
            box.value = await Self.run(lines: lines, source: source, target: target)
            done.signal()
        }
        if done.wait(timeout: .now() + 60) == .timedOut {
            return answer(["status": "error", "message": "the translation took too long"])
        }
        return box.value
    }

    private final class Box: @unchecked Sendable { var value = "" }

    static func base(_ code: String) -> String {
        String(code.split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? Substring(code)).lowercased()
    }

    private static func run(lines: [String], source: String?, target: String) async -> String {
        var src = source
        if src == nil || src!.isEmpty {
            let r = NLLanguageRecognizer()
            r.processString(lines.joined(separator: "\n"))
            src = r.dominantLanguage?.rawValue
        }
        guard let src else { return answer(["status": "undetermined"]) }
        if base(src) == base(target) { return answer(["status": "same", "source": src]) }
        let from = Locale.Language(identifier: src), to = Locale.Language(identifier: target)
        let status = await LanguageAvailability().status(from: from, to: to)
        switch status {
        case .installed: break
        case .supported:
            lastNeedsDownload = (src, target)
            return answer(["status": "needs_download", "source": src])
        case .unsupported: return answer(["status": "unsupported", "source": src])
        @unknown default: return answer(["status": "unsupported", "source": src])
        }
        let session = TranslationSession(installedSource: from, target: to)
        let requests = lines.enumerated().compactMap { i, l in
            l.trimmingCharacters(in: .whitespaces).isEmpty ? nil : TranslationSession.Request(sourceText: l, clientIdentifier: String(i))
        }
        do {
            let responses = try await session.translations(from: requests)
            var out = Array(repeating: "", count: lines.count)
            for r in responses {
                if let id = r.clientIdentifier, let i = Int(id), i < out.count { out[i] = r.targetText }
            }
            return answer(["status": "ok", "source": src, "lines": out])
        } catch {
            return answer(["status": "error", "source": src, "message": error.localizedDescription])
        }
    }

    private static func answer(_ d: [String: Any]) -> String {
        (try? JSONSerialization.data(withJSONObject: d)).flatMap { String(data: $0, encoding: .utf8) } ?? "{}"
    }

    private func answer(_ d: [String: Any]) -> String { Self.answer(d) }
}

#if DEBUG
/// Debug builds: what the Translation framework offers on this device (`-probeTranslate`),
/// written to Documents/probe.log.
enum TranslateProbe {
    static func run() async {
        var out: [String] = []
        let a = LanguageAvailability()
        let langs = await a.supportedLanguages
        out.append("supported languages: \(langs.count) — \(langs.map { $0.maximalIdentifier }.sorted().joined(separator: ", "))")
        for (s, t) in [("en", "es"), ("ja", "en"), ("en-US", "es-ES"), ("ja-JP", "en-US"), ("ja", "en-US"), ("en-US", "es"), ("es-ES", "en-US")] {
            let st = await a.status(from: Locale.Language(identifier: s), to: Locale.Language(identifier: t))
            out.append("status \(s)→\(t): \(st)")
        }
        if let st = try? await a.status(for: "東京の夜は静かに過ぎていく", to: Locale.Language(identifier: "en")) { out.append("status (detected ja text)→en: \(st)") }
        for (f, t) in [("en", "es"), ("en-US", "es-ES"), ("ja-JP", "en-US")] {
            let r = await AppleTranslatorProbeHelper.translate(f.hasPrefix("ja") ? ["こんにちは", "夜は静か"] : ["Good morning", "The night is quiet"], from: f, to: t)
            out.append("translate \(f)→\(t): \(r)")
        }
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("probe.log")
        try? out.joined(separator: "\n").write(to: docs, atomically: true, encoding: .utf8)
    }
}

enum AppleTranslatorProbeHelper {
    static func translate(_ lines: [String], from: String, to: String) async -> String {
        let session = TranslationSession(installedSource: Locale.Language(identifier: from), target: Locale.Language(identifier: to))
        do {
            let rs = try await session.translations(from: lines.map { TranslationSession.Request(sourceText: $0) })
            return rs.map(\.targetText).joined(separator: " | ")
        } catch {
            return "error: \(error) — \(error.localizedDescription)"
        }
    }
}
#endif
