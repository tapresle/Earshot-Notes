// Draws the Earshot app icon: white sound wave arcs on a violet gradient, in
// the macOS icon grid's rounded square (824 points with 100-point margins on
// a 1024 x 1024 canvas). Regenerate every icon size with:
//
//   swift src-tauri/icons/source/draw-icon.swift src-tauri/icons/source/icon.png
//   npx tauri icon src-tauri/icons/source/icon.png
//
// then delete the android and ios folders and 64x64.png that `tauri icon`
// also writes, because the app doesn't target them.
//
// The arcs come from the Icon Composer document's SVG layer. The outline comes
// from the transparency of Earshot-1024.png, Icon Composer's export of that
// document, so the corners match Apple's shape exactly; macOS 26 puts icons
// whose shape doesn't match on a gray background.
import AppKit
import UniformTypeIdentifiers

let size = 1024
let output = CommandLine.arguments.dropFirst().first ?? "icon.png"
let source = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
let glyphURL = source.appendingPathComponent("Earshot.icon/Assets/EarshotIcon-white.svg")
let shapeURL = source.appendingPathComponent("Earshot-1024.png")

func rgb(_ hex: UInt32) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255, alpha: 1)
}

func context() -> CGContext {
    CGContext(
        data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
}

/// The arcs as a bitmap cropped to their visible pixels.
func glyph() -> CGImage {
    guard let svg = NSImage(contentsOf: glyphURL) else { fatalError("couldn't read \(glyphURL.path)") }
    var rect = CGRect(x: 0, y: 0, width: 2048, height: 2048 * svg.size.height / svg.size.width)
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

// Full-bleed artwork, in bottom-left coordinates, matching the Icon Composer export.
let art = context()
let gradient = CGGradient(
    colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
    colors: [rgb(0x9933FF), rgb(0x9933FF), rgb(0x7000FF), rgb(0x7000FF)] as CFArray,
    locations: [0, 0.1, 0.67, 1])!
art.drawLinearGradient(
    gradient, start: CGPoint(x: 0, y: size), end: CGPoint(x: 0, y: 0), options: [])

// The arcs span x 152 to 872 and y 156 to 868 (from the top) in the export.
let arcs = glyph()
let target = CGRect(x: 152, y: 156, width: 720, height: 712)
let scale = min(target.width / CGFloat(arcs.width), target.height / CGFloat(arcs.height))
let drawn = CGSize(width: CGFloat(arcs.width) * scale, height: CGFloat(arcs.height) * scale)
art.interpolationQuality = .high
art.draw(arcs, in: CGRect(
    x: target.midX - drawn.width / 2, y: CGFloat(size) - target.midY - drawn.height / 2,
    width: drawn.width, height: drawn.height))

// Cut out Apple's rounded square.
guard let shapeSource = CGImageSourceCreateWithURL(shapeURL as CFURL, nil),
    let shape = CGImageSourceCreateImageAtIndex(shapeSource, 0, nil)
else { fatalError("couldn't read \(shapeURL.path)") }
art.setBlendMode(.destinationIn)
art.draw(shape, in: CGRect(x: 0, y: 0, width: size, height: size))

// Place it on the macOS grid.
let icon = context()
icon.interpolationQuality = .high
icon.draw(art.makeImage()!, in: CGRect(x: 100, y: 100, width: 824, height: 824))

let url = URL(fileURLWithPath: output) as CFURL
let destination = CGImageDestinationCreateWithURL(url, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, icon.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else {
    fatalError("couldn't write \(output)")
}
print("wrote \(output)")
