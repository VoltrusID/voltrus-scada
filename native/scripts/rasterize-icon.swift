// Render an SVG into a square PNG with AppKit, which reads SVG natively.
// Usage: rasterize-icon <input.svg> <output.png> [size, default 1024]
import AppKit

let arguments = CommandLine.arguments
guard arguments.count >= 3 else {
    FileHandle.standardError.write("usage: rasterize-icon <input.svg> <output.png> [size]\n".data(using: .utf8)!)
    exit(2)
}
let size = arguments.count > 3 ? Int(arguments[3]) ?? 1024 : 1024
guard let image = NSImage(contentsOf: URL(fileURLWithPath: arguments[1])) else {
    FileHandle.standardError.write("cannot read \(arguments[1])\n".data(using: .utf8)!)
    exit(1)
}
guard let bitmap = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0
) else { exit(1) }
bitmap.size = NSSize(width: size, height: size)
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
NSGraphicsContext.restoreGraphicsState()
guard let png = bitmap.representation(using: .png, properties: [:]) else { exit(1) }
do {
    try png.write(to: URL(fileURLWithPath: arguments[2]))
} catch {
    FileHandle.standardError.write("cannot write \(arguments[2]): \(error)\n".data(using: .utf8)!)
    exit(1)
}
