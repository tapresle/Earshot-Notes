// Draws the Earshot app icon: a microphone with sound waves on an indigo
// rounded square, at 1024 x 1024. Regenerate every icon size with:
//
//   swift src-tauri/icons/source/draw-icon.swift src-tauri/icons/source/icon.png
//   npx tauri icon src-tauri/icons/source/icon.png
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size = 1024
let output = CommandLine.arguments.dropFirst().first ?? "icon.png"

let ctx = CGContext(
    data: nil, width: size, height: size, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpace(name: CGColorSpace.sRGB)!,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

// Work in top-left coordinates.
ctx.translateBy(x: 0, y: CGFloat(size))
ctx.scaleBy(x: 1, y: -1)

func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
}

// Background: the macOS icon grid's rounded square (824 points with 100-point margins).
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let tilePath = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)
ctx.saveGState()
ctx.addPath(tilePath)
ctx.clip()
let gradient = CGGradient(
    colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: [rgb(0x6366F1), rgb(0x3B1FA8)] as CFArray,
    locations: [0, 1])!
ctx.drawLinearGradient(gradient, start: CGPoint(x: 200, y: 100), end: CGPoint(x: 824, y: 924), options: [])
ctx.restoreGState()

let stroke: CGFloat = 38
ctx.setLineCap(.round)
ctx.setLineWidth(stroke)

// Microphone capsule.
let capsule = CGRect(x: 512 - 78, y: 322, width: 156, height: 282)
ctx.setFillColor(rgb(0xFFFFFF))
ctx.addPath(CGPath(roundedRect: capsule, cornerWidth: 78, cornerHeight: 78, transform: nil))
ctx.fillPath()

// Stand: a cradle under the capsule, a stem, and a base.
ctx.setStrokeColor(rgb(0xFFFFFF))
ctx.addArc(center: CGPoint(x: 512, y: 506), radius: 138, startAngle: 0, endAngle: .pi, clockwise: false)
ctx.strokePath()
ctx.move(to: CGPoint(x: 512, y: 644))
ctx.addLine(to: CGPoint(x: 512, y: 730))
ctx.move(to: CGPoint(x: 432, y: 730))
ctx.addLine(to: CGPoint(x: 592, y: 730))
ctx.strokePath()

// Sound waves on both sides, fading outward.
let waveCenter = CGPoint(x: 512, y: 476)
let spread = CGFloat.pi / 5
for (radius, alpha) in [(CGFloat(232), CGFloat(0.9)), (CGFloat(318), CGFloat(0.55))] {
    ctx.setStrokeColor(rgb(0xFFFFFF, alpha))
    ctx.addArc(center: waveCenter, radius: radius, startAngle: -spread, endAngle: spread, clockwise: false)
    ctx.strokePath()
    ctx.addArc(center: waveCenter, radius: radius, startAngle: .pi - spread, endAngle: .pi + spread, clockwise: false)
    ctx.strokePath()
}

let image = ctx.makeImage()!
let url = URL(fileURLWithPath: output) as CFURL
let destination = CGImageDestinationCreateWithURL(url, UTType.png.identifier as CFString, 1, nil)!
CGImageDestinationAddImage(destination, image, nil)
guard CGImageDestinationFinalize(destination) else {
    fatalError("couldn't write \(output)")
}
print("wrote \(output)")
