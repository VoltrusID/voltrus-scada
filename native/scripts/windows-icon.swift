// Render the app icon SVG into a Windows .ico with AppKit, which reads SVG
// natively: 32-bit BMP images up to 64 px (what every Windows icon reader,
// rc.exe and Inno Setup take) and a PNG image at 256 px. The macOS artwork
// keeps Apple's 100/1024 margin around the tile; Windows icons fill their
// square, so the tile is scaled to 96 % of each image.
// Usage: swift native/scripts/windows-icon.swift <input.svg> <output.ico>
// The result is committed (native/packaging/windows/voltrus.ico): the Windows
// build embeds it and the installer shows it. Re-run after changing the SVG.
import AppKit

let arguments = CommandLine.arguments
guard arguments.count == 3 else {
    FileHandle.standardError.write("usage: windows-icon <input.svg> <output.ico>\n".data(using: .utf8)!)
    exit(2)
}
guard let image = NSImage(contentsOf: URL(fileURLWithPath: arguments[1])) else {
    FileHandle.standardError.write("cannot read \(arguments[1])\n".data(using: .utf8)!)
    exit(1)
}

// The tile's square in the 1024-unit artwork (AppIcon.svg's rounded rect).
let tileOrigin = 100.0, tileSize = 824.0, artwork = 1024.0

func render(_ size: Int) -> NSBitmapImageRep {
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: size * 4, bitsPerPixel: 32
    ) else { exit(1) }
    bitmap.size = NSSize(width: size, height: size)
    let side = Double(size)
    let scale = side * 0.96 / tileSize
    let origin = side * 0.02 - tileOrigin * scale
    NSGraphicsContext.saveGraphicsState()
    let context = NSGraphicsContext(bitmapImageRep: bitmap)
    context?.imageInterpolation = .high
    NSGraphicsContext.current = context
    image.draw(in: NSRect(x: origin, y: origin, width: artwork * scale, height: artwork * scale))
    NSGraphicsContext.restoreGraphicsState()
    return bitmap
}

func append<T: FixedWidthInteger>(_ value: T, to data: inout Data) {
    withUnsafeBytes(of: value.littleEndian) { data.append(contentsOf: $0) }
}

// A DIB as an icon image: BITMAPINFOHEADER (height doubled for the mask),
// bottom-up straight-alpha BGRA rows, then the 1-bit AND mask (set where the
// pixel is fully transparent).
func bmp(_ bitmap: NSBitmapImageRep, _ size: Int) -> Data {
    guard let pixels = bitmap.bitmapData else { exit(1) }
    let stride = bitmap.bytesPerRow
    let maskStride = (size + 31) / 32 * 4
    var data = Data()
    append(UInt32(40), to: &data)
    append(Int32(size), to: &data)
    append(Int32(size * 2), to: &data)
    append(UInt16(1), to: &data)
    append(UInt16(32), to: &data)
    append(UInt32(0), to: &data)
    append(UInt32(size * size * 4 + maskStride * size), to: &data)
    for _ in 0..<4 { append(UInt32(0), to: &data) }
    var mask = Data(count: maskStride * size)
    for row in 0..<size {
        let y = size - 1 - row
        for x in 0..<size {
            let p = pixels + y * stride + x * 4
            let (r, g, b, a) = (Int(p[0]), Int(p[1]), Int(p[2]), Int(p[3]))
            // AppKit draws premultiplied; icons carry straight alpha.
            let straight = { (c: Int) -> UInt8 in a == 0 ? 0 : UInt8(min(255, (c * 255 + a / 2) / a)) }
            data.append(contentsOf: [straight(b), straight(g), straight(r), UInt8(a)])
            if a == 0 { mask[row * maskStride + x / 8] |= UInt8(0x80 >> (x % 8)) }
        }
    }
    data.append(mask)
    return data
}

let sizes = [256, 64, 48, 40, 32, 24, 20, 16]
let images: [Data] = sizes.map { size in
    let bitmap = render(size)
    if size == 256 {
        guard let png = bitmap.representation(using: .png, properties: [:]) else { exit(1) }
        return png
    }
    return bmp(bitmap, size)
}

var ico = Data()
append(UInt16(0), to: &ico)
append(UInt16(1), to: &ico)
append(UInt16(sizes.count), to: &ico)
var offset = 6 + 16 * sizes.count
for (size, data) in zip(sizes, images) {
    ico.append(UInt8(size == 256 ? 0 : size))
    ico.append(UInt8(size == 256 ? 0 : size))
    ico.append(0)
    ico.append(0)
    append(UInt16(1), to: &ico)
    append(UInt16(32), to: &ico)
    append(UInt32(data.count), to: &ico)
    append(UInt32(offset), to: &ico)
    offset += data.count
}
for data in images { ico.append(data) }
do {
    try ico.write(to: URL(fileURLWithPath: arguments[2]))
} catch {
    FileHandle.standardError.write("cannot write \(arguments[2]): \(error)\n".data(using: .utf8)!)
    exit(1)
}
