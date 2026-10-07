// Draws the menu bar icons: the app icon's sound wave arcs as black template
// images, which macOS tints for the menu bar's appearance. The recording
// variant adds a filled dot. Regenerate with:
//
//   swift src-tauri/icons/source/draw-tray-icons.swift src-tauri/icons/tray
//
// Each image is 36 x 36 pixels, the 2x size of an 18-point menu bar icon.
import AppKit
import UniformTypeIdentifiers

let size = 36
let outputDir = URL(fileURLWithPath: CommandLine.arguments.dropFirst().first ?? ".")
let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let glyphURL = source.appendingPathComponent("Earshot.icon/Assets/EarshotIcon-white.svg")

/// The thinnest stroke the arcs get, in the SVG's units. The app icon's outer
/// arcs are thinner (1 to 1.5), which fades away at menu bar size; the inner
/// arcs are left alone, because thickening them closes the gaps between them.
let minimumStroke = 2.0

/// The arcs, with thin strokes thickened, as a bitmap cropped to their visible pixels.
func glyph() -> CGImage {
    guard var text = try? String(contentsOf: glyphURL, encoding: .utf8) else {
        fatalError("couldn't read \(glyphURL.path)")
    }
    let widths = try! NSRegularExpression(pattern: "stroke-width:([0-9.]+)")
    for match in widths.matches(in: text, range: NSRange(text.startIndex..., in: text)).reversed() {
        let range = Range(match.range(at: 1), in: text)!
        text.replaceSubrange(range, with: String(max(Double(text[range])!, minimumStroke)))
    }
    guard let svg = NSImage(data: Data(text.utf8)) else { fatalError("couldn't parse \(glyphURL.path)") }
    var rect = CGRect(x: 0, y: 0, width: 1024, height: 1024 * svg.size.height / svg.size.width)
    let image = svg.cgImage(forProposedRect: &rect, context: nil, hints: nil)!
    let (w, h) = (image.width, image.height)
    var pixels = [UInt8](repeating: 0, count: w * h * 4)
    let scan = CGContext(
        data: &pixels, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    scan.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    var (minX, minY, maxX, maxY) = (w, h, -1, -1)
    for y in 0..<h {
        for x in 0..<w where pixels[(y * w + x) * 4 + 3] > 0 {
            (minX, minY, maxX, maxY) = (min(minX, x), min(minY, y), max(maxX, x), max(maxY, y))
        }
    }
    return image.cropping(to: CGRect(x: minX, y: minY, width: maxX - minX + 1, height: maxY - minY + 1))!
}

func draw(recording: Bool, to name: String) {
    let ctx = CGContext(
        data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    ctx.interpolationQuality = .high
    let arcs = glyph()
    // Arcs fill the icon when idle, and make room for the dot when recording.
    let box = recording ? CGRect(x: 1, y: 7, width: 28, height: 28) : CGRect(x: 2, y: 2, width: 32, height: 32)
    let scale = min(box.width / CGFloat(arcs.width), box.height / CGFloat(arcs.height))
    let drawn = CGSize(width: CGFloat(arcs.width) * scale, height: CGFloat(arcs.height) * scale)
    let at = CGRect(x: box.midX - drawn.width / 2, y: box.midY - drawn.height / 2, width: drawn.width, height: drawn.height)
    // The SVG is white; draw it as a mask so the template is black.
    ctx.saveGState()
    ctx.clip(to: at, mask: arcs)
    ctx.setFillColor(CGColor(gray: 0, alpha: 1))
    ctx.fill(at)
    ctx.restoreGState()
    if recording {
        ctx.setFillColor(CGColor(gray: 0, alpha: 1))
        ctx.fillEllipse(in: CGRect(x: 24, y: 1, width: 11, height: 11))
    }
    let url = outputDir.appendingPathComponent(name)
    guard let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fatalError("couldn't write \(url.path)") }
    CGImageDestinationAddImage(dest, ctx.makeImage()!, nil)
    guard CGImageDestinationFinalize(dest) else { fatalError("couldn't write \(url.path)") }
}

draw(recording: false, to: "idle.png")
draw(recording: true, to: "recording.png")
