// Draws Glass's icon: a dark glass tile with copper level bars.
// swift packaging/icon/make-icon.swift <out-dir>  →  PNGs at every icon size
// (packaging/icon/png). Glass.icns is made from them with iconutil, which wants
// them named by point size:
//   mkdir Glass.iconset; for s in 16 32 128 256 512; do
//     cp png/glass-$s.png Glass.iconset/icon_${s}x${s}.png
//     cp png/glass-$((s * 2)).png Glass.iconset/icon_${s}x${s}@2x.png; done
//   iconutil -c icns Glass.iconset -o Glass.icns
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "."
let accentTop = NSColor(srgbRed: 0xF0/255, green: 0xBE/255, blue: 0x9F/255, alpha: 1)
let accent = NSColor(srgbRed: 0xD8/255, green: 0x9B/255, blue: 0x7A/255, alpha: 1)

func draw(_ px: Int) -> Data? {
    let s = CGFloat(px)
    guard let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0) else { return nil }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    // The tile: macOS icon grid (824 of 1024, centred), rounded like Big Sur icons.
    let inset = s * 100 / 1024
    let tile = NSRect(x: inset, y: inset, width: s - inset * 2, height: s - inset * 2)
    let radius = tile.width * 0.225
    let path = NSBezierPath(roundedRect: tile, xRadius: radius, yRadius: radius)
    // Soft shadow under the tile.
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.35)
    shadow.shadowBlurRadius = s * 0.025
    shadow.shadowOffset = NSSize(width: 0, height: -s * 0.012)
    NSGraphicsContext.saveGraphicsState()
    shadow.set()
    NSColor(srgbRed: 0.09, green: 0.10, blue: 0.12, alpha: 1).setFill()
    path.fill()
    NSGraphicsContext.restoreGraphicsState()
    // Glass: a deep blue-grey gradient, a frosted highlight on top, a fine rim.
    NSGraphicsContext.saveGraphicsState()
    path.addClip()
    NSGradient(colors: [NSColor(srgbRed: 0.16, green: 0.18, blue: 0.23, alpha: 1), NSColor(srgbRed: 0.06, green: 0.07, blue: 0.09, alpha: 1)])!.draw(in: tile, angle: -90)
    let glow = NSBezierPath(ovalIn: NSRect(x: tile.minX - tile.width * 0.25, y: tile.midY, width: tile.width * 1.5, height: tile.height * 0.9))
    NSGradient(colors: [NSColor.white.withAlphaComponent(0.13), NSColor.white.withAlphaComponent(0.0)])!.draw(in: glow, angle: -90)
    // The level bars, as in Glass's lyrics and visualizer.
    let heights: [CGFloat] = [0.34, 0.62, 0.46, 0.78, 0.40]
    let barW = tile.width * 0.085
    let gap = tile.width * 0.062
    let total = CGFloat(heights.count) * barW + CGFloat(heights.count - 1) * gap
    var x = tile.midX - total / 2
    let base = tile.minY + tile.height * 0.22
    for h in heights {
        let bh = tile.height * 0.62 * h
        let bar = NSBezierPath(roundedRect: NSRect(x: x, y: base, width: barW, height: bh), xRadius: barW / 2, yRadius: barW / 2)
        NSGradient(colors: [accent, accentTop])!.draw(in: bar, angle: 90)
        x += barW + gap
    }
    NSGraphicsContext.restoreGraphicsState()
    NSColor.white.withAlphaComponent(0.10).setStroke()
    path.lineWidth = max(1, s / 512)
    path.stroke()
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])
}

do {
    try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
    for px in [16, 32, 64, 128, 256, 512, 1024] {
        guard let d = draw(px) else { throw CocoaError(.fileWriteUnknown) }
        try d.write(to: URL(fileURLWithPath: "\(out)/glass-\(px).png"))
    }
} catch {
    FileHandle.standardError.write("make-icon: \(error.localizedDescription)\n".data(using: .utf8)!)
    exit(1)
}
