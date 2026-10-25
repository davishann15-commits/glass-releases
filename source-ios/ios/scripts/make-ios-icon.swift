// Glass's icon for iOS: the desktop icon's design (packaging/icon/make-icon.swift),
// drawn full-bleed and opaque, since iOS applies its own mask and rejects alpha.
// swift ios/scripts/make-ios-icon.swift <out.png>
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon-1024.png"
let accentTop = NSColor(srgbRed: 0xF0/255, green: 0xBE/255, blue: 0x9F/255, alpha: 1)
let accent = NSColor(srgbRed: 0xD8/255, green: 0x9B/255, blue: 0x7A/255, alpha: 1)
let px = 1024
let s = CGFloat(px)
// 32 bits per pixel with the alpha byte skipped: opaque, and a format Core Graphics
// can draw into (a 24-bit bitmap can't be drawn into: it came out black).
let cg = CGContext(data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(cgContext: cg, flipped: false)
let tile = NSRect(x: 0, y: 0, width: s, height: s)
NSGradient(colors: [NSColor(srgbRed: 0.16, green: 0.18, blue: 0.23, alpha: 1), NSColor(srgbRed: 0.06, green: 0.07, blue: 0.09, alpha: 1)])!.draw(in: tile, angle: -90)
let glow = NSBezierPath(ovalIn: NSRect(x: tile.minX - tile.width * 0.25, y: tile.midY, width: tile.width * 1.5, height: tile.height * 0.9))
NSGradient(colors: [NSColor.white.withAlphaComponent(0.13), NSColor.white.withAlphaComponent(0.0)])!.draw(in: glow, angle: -90)
// The same bars, scaled to the iOS grid (the desktop tile was 824 of 1024).
let inner = tile.insetBy(dx: s * 0.1, dy: s * 0.1)
let heights: [CGFloat] = [0.34, 0.62, 0.46, 0.78, 0.40]
let barW = inner.width * 0.085
let gap = inner.width * 0.062
let total = CGFloat(heights.count) * barW + CGFloat(heights.count - 1) * gap
var x = inner.midX - total / 2
let base = inner.minY + inner.height * 0.22
for h in heights {
    let bh = inner.height * 0.62 * h
    let bar = NSBezierPath(roundedRect: NSRect(x: x, y: base, width: barW, height: bh), xRadius: barW / 2, yRadius: barW / 2)
    NSGradient(colors: [accent, accentTop])!.draw(in: bar, angle: 90)
    x += barW + gap
}
NSGraphicsContext.restoreGraphicsState()
let rep = NSBitmapImageRep(cgImage: cg.makeImage()!)
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
