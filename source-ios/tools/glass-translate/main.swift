// glass-translate: Apple's on-device translation for Glass's lyrics (macOS 26+).
//
// Reads {"lines": [...], "source": "ja" or null, "target": "en"} on stdin and
// writes {"status": ..., "source": ..., "lines": [...]} on stdout. Status is
// "ok", "same" (already in the target language), "needs_download" (the pair is
// supported but not downloaded), "unsupported", "undetermined" or "error".
// Nothing leaves the computer.

import Foundation
import NaturalLanguage
import Translation

struct Input: Decodable {
    let lines: [String]
    let source: String?
    let target: String
}

func emit(_ obj: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: obj) {
        FileHandle.standardOutput.write(data)
    }
}

func base(_ s: String) -> String {
    String(s.split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? "").lowercased()
}

@main
struct Main {
    static func main() async {
        let data = FileHandle.standardInput.readDataToEndOfFile()
        guard let input = try? JSONDecoder().decode(Input.self, from: data) else {
            emit(["status": "error", "message": "bad input"])
            return
        }
        var source = input.source
        if source == nil {
            let r = NLLanguageRecognizer()
            r.processString(input.lines.joined(separator: "\n"))
            if let lang = r.dominantLanguage, lang != .undetermined {
                source = lang.rawValue
            }
        }
        guard let src = source else {
            emit(["status": "undetermined"])
            return
        }
        if base(src) == base(input.target) {
            emit(["status": "same", "source": base(src)])
            return
        }
        let from = Locale.Language(identifier: src)
        let to = Locale.Language(identifier: input.target)
        switch await LanguageAvailability().status(from: from, to: to) {
        case .installed:
            break
        case .supported:
            emit(["status": "needs_download", "source": base(src)])
            return
        default:
            emit(["status": "unsupported", "source": base(src)])
            return
        }
        let session = TranslationSession(installedSource: from, target: to)
        // Blank lines stay blank; the rest go in one batch, matched back by index.
        var requests: [TranslationSession.Request] = []
        for (i, line) in input.lines.enumerated() where !line.trimmingCharacters(in: .whitespaces).isEmpty {
            requests.append(TranslationSession.Request(sourceText: line, clientIdentifier: String(i)))
        }
        do {
            let responses = try await session.translations(from: requests)
            var out = Array(repeating: "", count: input.lines.count)
            for r in responses {
                if let id = r.clientIdentifier, let i = Int(id), i < out.count {
                    out[i] = r.targetText
                }
            }
            emit(["status": "ok", "source": base(src), "lines": out])
        } catch {
            emit(["status": "error", "source": base(src), "message": error.localizedDescription])
        }
    }
}
