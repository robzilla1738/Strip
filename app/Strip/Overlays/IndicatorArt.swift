import AppKit

enum PinEdge: Equatable {
    case left
    case right
}

struct IndicatorTile: Equatable {
    var title: String
    var count: Int
    var focused: Bool
    var pin: PinEdge?
    var windowID: UInt64
    var columnIndex: Int
}

enum IndicatorArt {
    static let tile: CGFloat = 40
    static let gap: CGFloat = 6
    static let pad: CGFloat = 8
    static let focus = NSColor(srgbRed: 76.0 / 255.0, green: 141.0 / 255.0, blue: 1, alpha: 1)
    static let pinColor = NSColor(srgbRed: 1, green: 0.72, blue: 0.2, alpha: 1)
    static let idle = NSColor(white: 1, alpha: 0.18)

    static func size(count: Int) -> NSSize {
        let width = pad * 2 + CGFloat(max(count, 1)) * tile + CGFloat(max(count - 1, 0)) * gap
        return NSSize(width: width, height: pad * 2 + tile)
    }

    static func frames(count: Int) -> [CGRect] {
        (0..<count).map { index in
            CGRect(x: pad + CGFloat(index) * (tile + gap), y: pad, width: tile, height: tile)
        }
    }

    static func render(_ tiles: [IndicatorTile]) -> NSBitmapImageRep {
        let size = size(count: tiles.count)
        let width = Int(size.width)
        let height = Int(size.height)
        let rep = NSBitmapImageRep(
            bitmapDataPlanes: nil,
            pixelsWide: width,
            pixelsHigh: height,
            bitsPerSample: 8,
            samplesPerPixel: 4,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: .deviceRGB,
            bytesPerRow: width * 4,
            bitsPerPixel: 32
        )!
        NSGraphicsContext.saveGraphicsState()
        let context = NSGraphicsContext(bitmapImageRep: rep)!
        NSGraphicsContext.current = context
        context.cgContext.translateBy(x: 0, y: CGFloat(height))
        context.cgContext.scaleBy(x: 1, y: -1)
        draw(tiles, in: context.cgContext)
        NSGraphicsContext.restoreGraphicsState()
        return rep
    }

    static func draw(_ tiles: [IndicatorTile], in context: CGContext) {
        for (index, tile) in tiles.enumerated() {
            let frame = frames(count: tiles.count)[index]
            context.setFillColor((tile.focused ? focus : idle).cgColor)
            context.fill(frame)
            if tile.count > 1 {
                let badge = CGRect(x: frame.maxX - 14, y: frame.minY + 4, width: 12, height: 12)
                context.setFillColor(NSColor.white.cgColor)
                context.fillEllipse(in: badge)
            }
            if let pin = tile.pin {
                let glyph = CGRect(
                    x: pin == .left ? frame.minX + 3 : frame.maxX - 9,
                    y: frame.midY - 3,
                    width: 6,
                    height: 6
                )
                context.setFillColor(pinColor.cgColor)
                context.fill(glyph)
            }
            let _ = tile.title
        }
    }
}

final class IndicatorTilesView: NSView {
    var tiles: [IndicatorTile] = [] {
        didSet { needsDisplay = true }
    }
    var onFocus: ((IndicatorTile) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?
    private var dragFrom: Int?
    private var preview: [IndicatorTile] = []

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        IndicatorArt.draw(preview.isEmpty ? tiles : preview, in: context)
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        dragFrom = hit(point)
        preview = tiles
    }

    override func mouseDragged(with event: NSEvent) {
        guard let dragFrom, let target = hit(convert(event.locationInWindow, from: nil)) else { return }
        preview = moved(tiles, from: dragFrom, to: target)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        defer {
            dragFrom = nil
            preview = []
            needsDisplay = true
        }
        guard let dragFrom else { return }
        if let target = hit(point), target != dragFrom {
            onMove?(tiles[dragFrom].columnIndex, tiles[target].columnIndex)
            return
        }
        if let index = hit(point) {
            onFocus?(tiles[index])
        }
    }

    private func hit(_ point: NSPoint) -> Int? {
        let shown = preview.isEmpty ? tiles : preview
        return IndicatorArt.frames(count: shown.count).firstIndex { $0.contains(point) }
    }

    private func moved(_ tiles: [IndicatorTile], from: Int, to: Int) -> [IndicatorTile] {
        guard tiles.indices.contains(from), tiles.indices.contains(to) else { return tiles }
        var copy = tiles
        let item = copy.remove(at: from)
        copy.insert(item, at: to)
        return copy
    }
}
