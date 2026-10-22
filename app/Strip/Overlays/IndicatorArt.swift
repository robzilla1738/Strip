import AppKit

enum PinEdge: Equatable {
    case left
    case right
}

struct IndicatorTile: Equatable {
    var title: String
    var bundleID: String
    var pid: Int32
    var count: Int
    var focused: Bool
    var pin: PinEdge?
    var windowID: UInt64
    var columnIndex: Int
}

enum IndicatorArt {
    static let tile: CGFloat = 48
    static let gap: CGFloat = 6
    static let pad: CGFloat = 8
    /// A true capsule, like the macOS 26 Dock and floating toolbars.
    static var panelRadius: CGFloat { (pad * 2 + tile) / 2 }
    static let tileRadius: CGFloat = 16
    static let focus = NSColor(srgbRed: 76.0 / 255.0, green: 141.0 / 255.0, blue: 1, alpha: 1)
    static let pinColor = NSColor(srgbRed: 1, green: 0.62, blue: 0.04, alpha: 1)
    static let idle = NSColor(white: 1, alpha: 0.10)
    static let idleLight = NSColor(white: 0, alpha: 0.06)

    /// The live accent, normalised to sRGB so it can be blended; falls back to the fixed blue.
    static var systemAccent: NSColor {
        NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? focus
    }

    static func size(count: Int) -> NSSize {
        let width = pad * 2 + CGFloat(max(count, 1)) * tile + CGFloat(max(count - 1, 0)) * gap
        return NSSize(width: width, height: pad * 2 + tile)
    }

    static func frames(count: Int) -> [CGRect] {
        (0..<count).map { index in
            CGRect(x: pad + CGFloat(index) * (tile + gap), y: pad, width: tile, height: tile)
        }
    }

    static func render(_ tiles: [IndicatorTile], dark: Bool = true, scale: CGFloat = 1, icon: (IndicatorTile) -> NSImage? = { _ in nil }) -> NSBitmapImageRep {
        let size = size(count: tiles.count)
        let width = Int(size.width * scale)
        let height = Int(size.height * scale)
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
        let cg = NSGraphicsContext(bitmapImageRep: rep)!.cgContext
        cg.translateBy(x: 0, y: CGFloat(height))
        cg.scaleBy(x: scale, y: -scale)
        NSGraphicsContext.current = NSGraphicsContext(cgContext: cg, flipped: true)
        draw(tiles, in: cg, dark: dark, icon: icon)
        NSGraphicsContext.current?.flushGraphics()
        NSGraphicsContext.restoreGraphicsState()
        rep.size = size
        return rep
    }

    /// Dock-style magnification falloff for the hovered tile and its neighbours.
    static func hoverScale(index: Int, hovered: Int?) -> CGFloat {
        guard let hovered else { return 1 }
        let distance = CGFloat(abs(index - hovered))
        guard distance < 2 else { return 1 }
        return 1 + 0.16 * cos(distance / 2 * .pi / 2)
    }

    /// Draws into a y-down context whose `NSGraphicsContext.current` wraps `context`.
    static func draw(
        _ tiles: [IndicatorTile],
        in context: CGContext,
        dark: Bool = true,
        lifted: Int? = nil,
        hovered: Int? = nil,
        accent: NSColor = focus,
        icon: (IndicatorTile) -> NSImage? = { _ in nil }
    ) {
        let frames = frames(count: tiles.count)
        for (index, tile) in tiles.enumerated() where index != lifted {
            let frame = frames[index]
            let scale = hoverScale(index: index, hovered: hovered)
            if scale == 1 {
                drawTile(tile, frame: frame, in: context, dark: dark, accent: accent, icon: icon(tile))
                continue
            }
            context.saveGState()
            context.translateBy(x: frame.midX, y: frame.midY)
            context.scaleBy(x: scale, y: scale)
            context.translateBy(x: -frame.midX, y: -frame.midY)
            drawTile(tile, frame: frame, in: context, dark: dark, accent: accent, icon: icon(tile))
            context.restoreGState()
        }
        guard let lifted, tiles.indices.contains(lifted) else { return }
        let frame = frames[lifted]
        context.saveGState()
        context.translateBy(x: frame.midX, y: frame.midY)
        context.scaleBy(x: 1.12, y: 1.12)
        context.translateBy(x: -frame.midX, y: -frame.midY)
        context.setAlpha(0.92)
        context.setShadow(offset: CGSize(width: 0, height: -3), blur: 8, color: NSColor(white: 0, alpha: 0.4).cgColor)
        context.beginTransparencyLayer(auxiliaryInfo: nil)
        drawTile(tiles[lifted], frame: frame, in: context, dark: dark, accent: accent, icon: icon(tiles[lifted]))
        context.endTransparencyLayer()
        context.restoreGState()
    }

    private static func drawTile(_ tile: IndicatorTile, frame: CGRect, in context: CGContext, dark: Bool, accent: NSColor, icon: NSImage?) {
        let shape = CGPath(roundedRect: frame, cornerWidth: tileRadius, cornerHeight: tileRadius, transform: nil)
        if tile.focused {
            drawLitTile(shape, frame: frame, in: context, dark: dark, accent: accent)
        } else if icon == nil {
            context.addPath(shape)
            context.setFillColor((dark ? idle : idleLight).cgColor)
            context.fillPath()
        }
        if let icon {
            context.saveGState()
            context.setShadow(offset: CGSize(width: 0, height: -1), blur: 3, color: NSColor(white: 0, alpha: dark ? 0.35 : 0.18).cgColor)
            icon.draw(
                in: frame.insetBy(dx: 6, dy: 6),
                from: .zero,
                operation: .sourceOver,
                fraction: 1,
                respectFlipped: true,
                hints: [.interpolation: NSImageInterpolation.high.rawValue]
            )
            context.restoreGState()
        } else {
            drawInitial(tile, frame: frame, dark: dark)
        }
        if tile.count > 1 {
            drawCount(tile.count, focused: tile.focused, frame: frame, in: context, dark: dark, accent: accent)
        }
        if let pin = tile.pin {
            drawPin(pin, frame: frame, in: context)
        }
    }

    /// The focused column sits on a raised piece of clear glass — a bright
    /// neutral pill with a specular rim and soft shadow, like a selected
    /// segment in macOS 26 — marked by an accent dot beneath, Dock-style.
    private static func drawLitTile(_ shape: CGPath, frame: CGRect, in context: CGContext, dark: Bool, accent: NSColor) {
        let space = CGColorSpace(name: CGColorSpace.sRGB)
        context.saveGState()
        context.setShadow(offset: CGSize(width: 0, height: -2), blur: 8, color: NSColor(white: 0, alpha: dark ? 0.45 : 0.25).cgColor)
        context.addPath(shape)
        context.setFillColor(NSColor(white: 1, alpha: dark ? 0.24 : 0.72).cgColor)
        context.fillPath()
        context.restoreGState()

        context.saveGState()
        context.addPath(shape)
        context.clip()
        let top = NSColor(white: 1, alpha: dark ? 0.34 : 0.9)
        let mid = (accent.blended(withFraction: dark ? 0.82 : 0.88, of: .white) ?? .white).withAlphaComponent(dark ? 0.28 : 0.75)
        let bottom = NSColor(white: dark ? 1 : 0.92, alpha: dark ? 0.14 : 0.55)
        if let gradient = CGGradient(colorsSpace: space, colors: [top.cgColor, mid.cgColor, bottom.cgColor] as CFArray, locations: [0, 0.5, 1]) {
            context.drawLinearGradient(gradient, start: CGPoint(x: frame.midX, y: frame.minY), end: CGPoint(x: frame.midX, y: frame.maxY), options: [])
        }
        context.restoreGState()

        let rim = frame.insetBy(dx: 0.75, dy: 0.75)
        context.saveGState()
        context.addPath(CGPath(roundedRect: rim, cornerWidth: tileRadius - 0.75, cornerHeight: tileRadius - 0.75, transform: nil))
        context.setLineWidth(1.5)
        context.replacePathWithStrokedPath()
        context.clip()
        let shine = [NSColor(white: 1, alpha: dark ? 0.85 : 0.95).cgColor, NSColor(white: 1, alpha: 0.15).cgColor, NSColor(white: 1, alpha: dark ? 0.4 : 0.5).cgColor] as CFArray
        if let gradient = CGGradient(colorsSpace: space, colors: shine, locations: [0, 0.6, 1]) {
            context.drawLinearGradient(gradient, start: CGPoint(x: frame.midX, y: frame.minY), end: CGPoint(x: frame.midX, y: frame.maxY), options: [])
        }
        context.restoreGState()

        let dot: CGFloat = 5
        context.setFillColor(accent.cgColor)
        context.fillEllipse(in: CGRect(x: frame.midX - dot / 2, y: frame.maxY + (pad - dot) / 2, width: dot, height: dot))
    }

    private static func drawInitial(_ tile: IndicatorTile, frame: CGRect, dark: Bool) {
        guard let letter = tile.title.trimmingCharacters(in: .whitespaces).first else { return }
        let color: NSColor = tile.focused ? (dark ? .white : NSColor(white: 0, alpha: 0.85)) : (dark ? NSColor(white: 1, alpha: 0.75) : NSColor(white: 0, alpha: 0.6))
        let text = NSAttributedString(string: String(letter).uppercased(), attributes: [
            .font: NSFont.systemFont(ofSize: 19, weight: .semibold),
            .foregroundColor: color,
        ])
        let size = text.size()
        text.draw(at: CGPoint(x: frame.midX - size.width / 2, y: frame.midY - size.height / 2))
    }

    /// Badges carry a soft drop shadow rather than an outline so they stay legible on any glass tint.
    private static func badgeShadow(_ context: CGContext) {
        context.setShadow(offset: CGSize(width: 0, height: -1), blur: 3, color: NSColor(white: 0, alpha: 0.35).cgColor)
    }

    private static func drawCount(_ count: Int, focused: Bool, frame: CGRect, in context: CGContext, dark: Bool, accent: NSColor) {
        let text = NSAttributedString(string: count > 99 ? "99+" : "\(count)", attributes: [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 10, weight: .bold),
            .foregroundColor: NSColor.white,
        ])
        let size = text.size()
        let height: CGFloat = 16
        let width = max(height, ceil(size.width) + 9)
        let badge = CGRect(x: frame.maxX - width + 4, y: frame.maxY - height + 4, width: width, height: height)
        context.saveGState()
        badgeShadow(context)
        context.addPath(CGPath(roundedRect: badge, cornerWidth: height / 2, cornerHeight: height / 2, transform: nil))
        context.setFillColor((focused ? accent : NSColor(white: dark ? 0.4 : 0.18, alpha: 0.95)).cgColor)
        context.fillPath()
        context.restoreGState()
        text.draw(at: CGPoint(x: badge.midX - size.width / 2, y: badge.midY - size.height / 2))
    }

    private static func drawPin(_ pin: PinEdge, frame: CGRect, in context: CGContext) {
        let diameter: CGFloat = 17
        let badge = CGRect(
            x: pin == .left ? frame.minX - 4 : frame.maxX - diameter + 4,
            y: frame.minY - 4,
            width: diameter,
            height: diameter
        )
        context.saveGState()
        badgeShadow(context)
        context.setFillColor(pinColor.cgColor)
        context.fillEllipse(in: badge)
        context.restoreGState()
        context.saveGState()
        context.translateBy(x: badge.midX, y: badge.midY)
        context.rotate(by: (pin == .left ? -40 : 40) * .pi / 180)
        if let glyph = pinGlyph {
            let size = glyph.size
            glyph.draw(
                in: CGRect(x: -size.width / 2, y: -size.height / 2, width: size.width, height: size.height),
                from: .zero,
                operation: .sourceOver,
                fraction: 1,
                respectFlipped: true,
                hints: nil
            )
        } else {
            context.setFillColor(NSColor.white.cgColor)
            context.fillEllipse(in: CGRect(x: -2.5, y: -4.5, width: 5, height: 5))
            context.fill(CGRect(x: -0.75, y: 0, width: 1.5, height: 4.5))
        }
        context.restoreGState()
    }

    private static let pinGlyph: NSImage? = {
        let config = NSImage.SymbolConfiguration(pointSize: 9, weight: .bold)
        guard let symbol = NSImage(systemSymbolName: "pin.fill", accessibilityDescription: nil)?
            .withSymbolConfiguration(config) else { return nil }
        return NSImage(size: symbol.size, flipped: false) { rect in
            symbol.draw(in: rect)
            NSColor.white.set()
            rect.fill(using: .sourceAtop)
            return true
        }
    }()

    static func roundedMask(radius: CGFloat) -> NSImage {
        let edge = radius * 2 + 1
        let image = NSImage(size: NSSize(width: edge, height: edge), flipped: false) { rect in
            NSColor.black.setFill()
            NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
            return true
        }
        image.capInsets = NSEdgeInsets(top: radius, left: radius, bottom: radius, right: radius)
        image.resizingMode = .stretch
        return image
    }
}

/// App icons keyed by pid, falling back to the bundle's icon on disk when the
/// process is gone or not yet registered.
final class AppIconCache {
    static let shared = AppIconCache()
    private var byPID: [Int32: NSImage?] = [:]
    private var byBundle: [String: NSImage?] = [:]

    func icon(for tile: IndicatorTile) -> NSImage? {
        if tile.pid > 0, let cached = byPID[tile.pid] {
            return cached
        }
        if byPID.count > 256 { byPID.removeAll() }
        let image = NSRunningApplication(processIdentifier: tile.pid)?.icon ?? bundleIcon(tile.bundleID)
        if tile.pid > 0 { byPID[tile.pid] = image }
        return image
    }

    private func bundleIcon(_ bundleID: String) -> NSImage? {
        guard !bundleID.isEmpty else { return nil }
        if let cached = byBundle[bundleID] { return cached }
        let image = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleID)
            .map { NSWorkspace.shared.icon(forFile: $0.path) }
        byBundle[bundleID] = image
        return image
    }
}

final class IndicatorTilesView: NSView, NSViewToolTipOwner {
    var tiles: [IndicatorTile] = [] {
        didSet {
            guard tiles != oldValue else { return }
            if dragFrom != nil, tiles.count != oldValue.count { endDrag() }
            if hovered != nil, !tiles.indices.contains(hovered!) { hovered = nil }
            refreshToolTips()
            needsDisplay = true
        }
    }
    private var hovered: Int? {
        didSet { if hovered != oldValue { needsDisplay = true } }
    }
    var onFocus: ((IndicatorTile) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?
    /// Only the pre-glass material needs a drawn hairline; Liquid Glass renders its own edge.
    var drawsOutline = false {
        didSet { if drawsOutline != oldValue { needsDisplay = true } }
    }
    private var dragFrom: Int?
    private var dragTarget: Int?
    private var dragging = false
    private var preview: [IndicatorTile] = []

    var isInteracting: Bool { dragFrom != nil }

    override var isFlipped: Bool { true }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseMoved, .mouseEnteredAndExited, .activeAlways], owner: self))
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override func mouseMoved(with event: NSEvent) {
        hovered = reduceMotion ? nil : hit(convert(event.locationInWindow, from: nil))
    }

    override func mouseExited(with event: NSEvent) {
        hovered = nil
    }

    private var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    private func refreshToolTips() {
        removeAllToolTips()
        for (index, frame) in IndicatorArt.frames(count: tiles.count).enumerated() {
            _ = addToolTip(frame, owner: self, userData: UnsafeMutableRawPointer(bitPattern: index + 1))
        }
    }

    func view(_ view: NSView, stringForToolTip tag: NSView.ToolTipTag, point: NSPoint, userData data: UnsafeMutableRawPointer?) -> String {
        guard let data else { return "" }
        let index = Int(bitPattern: data) - 1
        guard tiles.indices.contains(index) else { return "" }
        let tile = tiles[index]
        let base = tile.title.isEmpty ? "Column \(index + 1)" : tile.title
        return tile.count > 1 ? "\(base) — \(tile.count) windows" : base
    }

    override func setFrameSize(_ newSize: NSSize) {
        let changed = newSize != frame.size
        super.setFrameSize(newSize)
        if changed { needsDisplay = true }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        if drawsOutline {
            let radius = IndicatorArt.panelRadius - 0.5
            let outline = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: radius, yRadius: radius)
            (dark ? NSColor(white: 1, alpha: 0.14) : NSColor(white: 0, alpha: 0.10)).setStroke()
            outline.lineWidth = 1
            outline.stroke()
        }
        let icons = AppIconCache.shared
        IndicatorArt.draw(
            preview.isEmpty ? tiles : preview,
            in: context,
            dark: dark,
            lifted: dragging ? dragTarget : nil,
            hovered: dragging ? nil : hovered,
            accent: IndicatorArt.systemAccent,
            icon: { icons.icon(for: $0) }
        )
    }

    override func mouseDown(with event: NSEvent) {
        dragFrom = hit(convert(event.locationInWindow, from: nil))
        dragTarget = dragFrom
        dragging = false
        preview = []
    }

    override func mouseDragged(with event: NSEvent) {
        guard let dragFrom, tiles.indices.contains(dragFrom), tiles[dragFrom].pin == nil else { return }
        if let target = hit(convert(event.locationInWindow, from: nil)), tiles[target].pin == nil {
            dragTarget = target
        }
        dragging = true
        preview = moved(tiles, from: dragFrom, to: dragTarget ?? dragFrom)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        let from = dragFrom
        let target = dragTarget
        let wasDragging = dragging
        endDrag()
        guard let from, tiles.indices.contains(from) else { return }
        if wasDragging {
            guard let target, target != from, tiles.indices.contains(target) else { return }
            let source = tiles[from].columnIndex
            let destination = tiles[target].columnIndex
            if source >= 0, destination >= 0 {
                onMove?(source, destination)
            }
            return
        }
        if let index = hit(point) {
            onFocus?(tiles[index])
        }
    }

    private func endDrag() {
        let hadPreview = dragging
        dragFrom = nil
        dragTarget = nil
        dragging = false
        preview = []
        if hadPreview { needsDisplay = true }
    }

    private func hit(_ point: NSPoint) -> Int? {
        IndicatorArt.frames(count: tiles.count).firstIndex { $0.insetBy(dx: -IndicatorArt.gap / 2, dy: -IndicatorArt.pad).contains(point) }
    }

    private func moved(_ tiles: [IndicatorTile], from: Int, to: Int) -> [IndicatorTile] {
        guard tiles.indices.contains(from), tiles.indices.contains(to) else { return tiles }
        var copy = tiles
        let item = copy.remove(at: from)
        copy.insert(item, at: to)
        return copy
    }
}
