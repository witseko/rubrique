// Render the Linux app icon SVG to the PNG sizes the packages install.
// Usage: swift packaging/linux/icons.swift assets/icon/linux/rubrique.svg assets/icon/linux
// Writes rubrique-64.png, -128, -256 and -512 next to the SVG (macOS only: uses AppKit).
import AppKit

let arguments = CommandLine.arguments
guard arguments.count == 3, let svg = NSImage(contentsOfFile: arguments[1]) else {
    FileHandle.standardError.write("usage: icons.swift ICON.svg OUTPUT_DIR\n".data(using: .utf8)!)
    exit(1)
}

for size in [64, 128, 256, 512] {
    // A bitmap of exactly size x size pixels, so the PNG is not doubled on Retina screens.
    let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    NSGraphicsContext.current?.imageInterpolation = .high
    svg.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
    NSGraphicsContext.restoreGraphicsState()
    let png = bitmap.representation(using: .png, properties: [:])!
    try! png.write(to: URL(fileURLWithPath: "\(arguments[2])/rubrique-\(size).png"))
}
