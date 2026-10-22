import AppKit

final class OverlayController {
    private let indicator = IndicatorPanel()
    private let tabs = TabBarPanel()
    private let border = FocusBorderPanel()
    private var dimPanels: [DimPanel] = []
    private let screens = ScreenCache()
    private var screenObserver: NSObjectProtocol?
    private var tiles: [IndicatorTile] = []
    private var windows: [OverlayWindow] = []
    private var managing = true
    private var showIndicator = true
    private var focusBorder = false
    private var focusBorderWidth = 4.0
    private var focusBorderColor = "#4C8DFF"
    private var borderColor = color(hex: "#4C8DFF").cgColor
    private var focusBorderRadius = 10.0
    private var dimInactive = false
    private var dimInactiveOpacity = 0.25
    private var state: JSONValue?
    private var preferredDisplay: String?
    private var modifiersHeld = false
    private var phase = IndicatorPhase.hidden
    private var indicatorTarget: NSRect?
    private var fadeDeadline: Date?
    private var hideTimer: Timer?
    private var fadeGeneration = 0
    var onFocus: ((UInt64) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?
    var onTab: ((UInt64) -> Void)?

    private enum IndicatorPhase {
        case hidden
        case showing
        case fadingOut
    }

    private static let linger: TimeInterval = 1.2
    private static let fadeOutDuration: TimeInterval = 0.2
    private static let fadeInDuration: TimeInterval = 0.22
    private static let entranceDrop: CGFloat = 10

    private var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    init() {
        indicator.onFocus = { [weak self] tile in
            self?.onFocus?(tile.windowID)
        }
        indicator.onMove = { [weak self] from, to in
            self?.onMove?(from, to)
        }
        tabs.onTab = { [weak self] id in
            self?.onTab?(id)
        }
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.screensChanged()
        }
    }

    deinit {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        hideTimer?.invalidate()
    }

    func applyAppearance(_ document: ConfigDocument) {
        showIndicator = document.showIndicator
        focusBorder = document.focusBorder
        focusBorderWidth = document.focusBorderWidth
        if document.focusBorderColor != focusBorderColor {
            focusBorderColor = document.focusBorderColor
            borderColor = color(hex: focusBorderColor).cgColor
        }
        focusBorderRadius = document.focusBorderRadius
        dimInactive = document.dimInactive
        dimInactiveOpacity = document.dimInactiveOpacity
        if !showIndicator { hideIndicatorNow() }
        placeDecorations()
    }

    func apply(event: WireLine) {
        guard event.type == "event" else { return }
        switch event.event {
        case "indicator":
            if let display = event.data?["display"]?.string, display != preferredDisplay {
                preferredDisplay = display
                rebuild()
                placeDecorations()
            }
            let show = event.data?["show"]?.bool == true
            let hold = event.data?["hold"]?.bool == true
            presentIndicator(show: show, hold: hold)
        case "state":
            guard let data = event.data else { return }
            state = data
            rebuild()
            placeDecorations()
            if phase != .hidden { placeIndicator() }
            if modifiersHeld { presentIndicator(show: true, hold: true) }
        default:
            break
        }
    }

    /// Hides every overlay, e.g. when the engine connection drops.
    func reset() {
        state = nil
        tiles = []
        windows = []
        modifiersHeld = false
        indicator.update([])
        tabs.dismiss()
        hideIndicatorNow()
        placeDecorations()
    }

    private func rebuild() {
        let display = selectedDisplay()
        let strip = display?["strips"]?.array?.first
        managing = state?["managing"]?.bool ?? true
        let fresh = tilesFrom(strip)
        if fresh != tiles {
            tiles = fresh
            indicator.update(tiles)
        }
        windows = managing ? managedWindows(strip) : []
        if managing, let column = tabColumn(strip), let frame = column.frame {
            tabs.update(column)
            let height = TabStripView.height
            tabs.place(NSRect(x: frame.minX, y: screens.primaryHeight - (frame.minY + height), width: max(frame.width, 1), height: height))
            tabs.present()
        } else {
            tabs.dismiss()
        }
    }

    private func presentIndicator(show: Bool, hold: Bool) {
        if hold {
            modifiersHeld = true
        } else if !show {
            modifiersHeld = false
        }
        let sticky = hold || (show && modifiersHeld)
        guard showIndicator, show || hold, !tiles.isEmpty else {
            if !sticky { armFade() }
            return
        }
        placeIndicator()
        revealIndicator()
        if sticky {
            fadeDeadline = nil
        } else {
            armFade()
        }
    }

    private func revealIndicator() {
        switch phase {
        case .showing:
            return
        case .hidden:
            indicator.alphaValue = 0
            if !reduceMotion, let target = indicatorTarget {
                indicator.setFrame(target.offsetBy(dx: 0, dy: -Self.entranceDrop), display: false)
            }
            indicator.orderFrontRegardless()
        case .fadingOut:
            break
        }
        phase = .showing
        fadeGeneration += 1
        let target = indicatorTarget
        NSAnimationContext.runAnimationGroup { context in
            context.duration = reduceMotion ? 0.01 : Self.fadeInDuration
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            indicator.animator().alphaValue = 1
            if let target {
                indicator.animator().setFrame(target, display: true)
            }
        }
    }

    /// Extends the linger deadline; one timer re-arms itself instead of being recreated per event.
    private func armFade() {
        guard phase == .showing else { return }
        fadeDeadline = Date().addingTimeInterval(Self.linger)
        if hideTimer == nil { scheduleHideTimer(after: Self.linger) }
    }

    private func scheduleHideTimer(after interval: TimeInterval) {
        let timer = Timer(timeInterval: interval, repeats: false) { [weak self] _ in
            self?.hideTimerFired()
        }
        RunLoop.main.add(timer, forMode: .common)
        hideTimer = timer
    }

    private func hideTimerFired() {
        hideTimer = nil
        guard phase == .showing, let deadline = fadeDeadline else { return }
        let remaining = deadline.timeIntervalSinceNow
        if remaining > 0.02 {
            scheduleHideTimer(after: remaining)
            return
        }
        if indicator.isInteracting || NSMouseInRect(NSEvent.mouseLocation, indicator.frame, false) {
            armFade()
            return
        }
        fadeOut()
    }

    private func fadeOut() {
        phase = .fadingOut
        fadeDeadline = nil
        fadeGeneration += 1
        let generation = fadeGeneration
        let resting = indicatorTarget ?? indicator.frame
        NSAnimationContext.runAnimationGroup { context in
            context.duration = reduceMotion ? 0.01 : Self.fadeOutDuration
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            indicator.animator().alphaValue = 0
            if !reduceMotion {
                indicator.animator().setFrame(resting.offsetBy(dx: 0, dy: -Self.entranceDrop / 2), display: true)
            }
        } completionHandler: { [weak self] in
            guard let self, self.fadeGeneration == generation, self.phase == .fadingOut else { return }
            self.phase = .hidden
            self.indicator.orderOut(nil)
        }
    }

    private func hideIndicatorNow() {
        hideTimer?.invalidate()
        hideTimer = nil
        fadeDeadline = nil
        fadeGeneration += 1
        phase = .hidden
        indicator.orderOut(nil)
        indicator.alphaValue = 0
    }

    private func placeIndicator() {
        let size = IndicatorArt.size(count: tiles.count)
        guard let screen = screens.screen(for: preferredDisplay) else { return }
        let visible = screen.visibleFrame
        let rect = NSRect(
            x: (visible.minX + (visible.width - size.width) / 2).rounded(),
            y: visible.minY + 14,
            width: size.width,
            height: size.height
        )
        let targetChanged = indicatorTarget != rect
        indicatorTarget = rect
        if phase == .hidden || targetChanged {
            indicator.place(rect)
        }
    }

    private func placeDecorations() {
        let shown = windows.filter { $0.visible && $0.frame.width > 1 && $0.frame.height > 1 }
        if focusBorder, let focused = shown.first(where: { $0.focused }) {
            let inset = -CGFloat(focusBorderWidth) / 2
            border.place(screens.appKitRect(focused.frame).insetBy(dx: inset, dy: inset))
            border.update(color: borderColor, lineWidth: focusBorderWidth, radius: focusBorderRadius)
            border.present()
        } else {
            border.dismiss()
        }
        let dimmed = dimInactive ? shown.filter { !$0.focused } : []
        while dimPanels.count < dimmed.count { dimPanels.append(DimPanel()) }
        for (index, panel) in dimPanels.enumerated() {
            guard index < dimmed.count else {
                panel.dismiss()
                continue
            }
            panel.place(screens.appKitRect(dimmed[index].frame))
            panel.update(opacity: dimInactiveOpacity)
            panel.present()
        }
    }

    private func screensChanged() {
        screens.invalidate()
        rebuild()
        placeDecorations()
        if phase != .hidden { placeIndicator() }
    }

    private func selectedDisplay() -> JSONValue? {
        let displays = state?["displays"]?.array ?? []
        if let preferredDisplay,
           let match = displays.first(where: { $0["uuid"]?.string?.caseInsensitiveCompare(preferredDisplay) == .orderedSame }) {
            return match
        }
        return displays.first
    }
}

/// Display UUID and primary-height lookups are comparatively slow; they only change with the screen layout.
private final class ScreenCache {
    private var byUUID: [String: NSScreen]?
    private var primary: CGFloat?

    func invalidate() {
        byUUID = nil
        primary = nil
    }

    var primaryHeight: CGFloat {
        if let primary { return primary }
        let screens = NSScreen.screens
        let height = screens.first { $0.frame.origin == .zero }?.frame.height ?? screens.first?.frame.height ?? 0
        primary = height
        return height
    }

    func screen(for uuid: String?) -> NSScreen? {
        guard let uuid else { return NSScreen.screens.first }
        if byUUID == nil {
            var map: [String: NSScreen] = [:]
            for screen in NSScreen.screens {
                guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else { continue }
                map[displayUUID(CGDirectDisplayID(number.uint32Value)).uppercased()] = screen
            }
            byUUID = map
        }
        return byUUID?[uuid.uppercased()] ?? NSScreen.screens.first
    }

    /// Engine frames are top-left-origin global coordinates.
    func appKitRect(_ frame: CGRect) -> NSRect {
        NSRect(x: frame.minX, y: primaryHeight - frame.maxY, width: frame.width, height: frame.height)
    }
}

func color(hex: String) -> NSColor {
    var text = hex.trimmingCharacters(in: .whitespacesAndNewlines)
    if text.hasPrefix("#") { text.removeFirst() }
    guard text.count == 6, let value = UInt32(text, radix: 16) else {
        return NSColor(srgbRed: 76.0 / 255.0, green: 141.0 / 255.0, blue: 1, alpha: 1)
    }
    return NSColor(
        srgbRed: CGFloat((value >> 16) & 0xff) / 255,
        green: CGFloat((value >> 8) & 0xff) / 255,
        blue: CGFloat(value & 0xff) / 255,
        alpha: 1
    )
}

private struct OverlayWindow {
    var frame: CGRect
    var focused: Bool
    var visible: Bool
}

private let overlayLevel = NSWindow.Level.statusBar
/// Above the border and dim panels so ordering never needs to be re-asserted.
private let chromeLevel = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)

private extension NSWindow {
    func configureOverlay(level: NSWindow.Level) {
        self.level = level
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        animationBehavior = .none
    }

    func place(_ rect: NSRect) {
        guard frame != rect else { return }
        let resized = frame.size != rect.size
        setFrame(rect, display: false)
        if resized && hasShadow { invalidateShadow() }
    }

    func present() {
        if !isVisible { orderFrontRegardless() }
    }

    func dismiss() {
        if isVisible { orderOut(nil) }
    }

    /// Wraps `content` in Liquid Glass on macOS 26+, otherwise in a masked popover material.
    /// Returns whether glass is used. Glass renders its own shadow, so the window shadow is
    /// turned off there to avoid doubling it. The bottom strip uses clear glass
    /// so the desktop refracts through it like the macOS 26 Dock.
    @discardableResult
    func installChromeBackground(around content: NSView, radius: CGFloat, clear: Bool = false) -> Bool {
        let bounds = NSRect(origin: .zero, size: frame.size)
        content.frame = bounds
        content.autoresizingMask = [.width, .height]
        if #available(macOS 26, *) {
            let glass = NSGlassEffectView(frame: bounds)
            glass.style = clear ? .clear : .regular
            glass.cornerRadius = radius
            glass.autoresizingMask = [.width, .height]
            glass.contentView = content
            contentView = glass
            hasShadow = false
            return true
        }
        let effect = NSVisualEffectView(frame: bounds)
        effect.material = .popover
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.maskImage = IndicatorArt.roundedMask(radius: radius)
        effect.autoresizingMask = [.width, .height]
        effect.addSubview(content)
        contentView = effect
        hasShadow = true
        return false
    }
}

private var glassAvailable: Bool {
    if #available(macOS 26, *) { return true }
    return false
}

private final class FocusBorderPanel: NSPanel {
    private let box = BorderBox()

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isFloatingPanel = true
        configureOverlay(level: overlayLevel)
        hasShadow = false
        ignoresMouseEvents = true
        box.autoresizingMask = [.width, .height]
        contentView = box
    }

    func update(color: CGColor, lineWidth: CGFloat, radius: CGFloat) {
        box.update(color: color, lineWidth: lineWidth, radius: radius)
    }
}

/// A shape layer keeps the stroke vector-crisp and avoids a CPU redraw on every frame change.
private final class BorderBox: NSView {
    private let shape = CAShapeLayer()
    private var color = NSColor.systemBlue.cgColor
    private var lineWidth: CGFloat = 4
    private var radius: CGFloat = 10

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        shape.fillColor = nil
        shape.lineJoin = .round
        layer?.addSublayer(shape)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func update(color: CGColor, lineWidth: CGFloat, radius: CGFloat) {
        guard color != self.color || lineWidth != self.lineWidth || radius != self.radius else { return }
        self.color = color
        self.lineWidth = lineWidth
        self.radius = radius
        updatePath()
    }

    override func setFrameSize(_ newSize: NSSize) {
        let changed = newSize != frame.size
        super.setFrameSize(newSize)
        if changed { updatePath() }
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        shape.contentsScale = window?.backingScaleFactor ?? 2
    }

    private func updatePath() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        shape.frame = bounds
        let rect = bounds.insetBy(dx: lineWidth / 2, dy: lineWidth / 2)
        if rect.width > 0, rect.height > 0 {
            let corner = max(0, min(radius, rect.width / 2, rect.height / 2))
            shape.path = CGPath(roundedRect: rect, cornerWidth: corner, cornerHeight: corner, transform: nil)
        } else {
            shape.path = nil
        }
        shape.lineWidth = lineWidth
        shape.strokeColor = color
        CATransaction.commit()
    }
}

/// The shade is the window's own background; opacity is the window alpha, so nothing redraws.
private final class DimPanel: NSPanel {
    private var opacity: Double = -1

    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
        isFloatingPanel = true
        configureOverlay(level: overlayLevel)
        backgroundColor = .black
        hasShadow = false
        ignoresMouseEvents = true
    }

    func update(opacity: Double) {
        guard opacity != self.opacity else { return }
        self.opacity = opacity
        alphaValue = CGFloat(opacity)
    }
}

private struct TabColumn: Equatable {
    var titles: [String]
    var active: Int
    var ids: [UInt64]
    var frame: CGRect?
}

func tileIsFocused(focus: String?, focusedColumn: Int, pin: PinEdge?, column: Int) -> Bool {
    switch focus {
    case "pinned_left": return pin == .left
    case "pinned_right": return pin == .right
    case "none": return false
    default: return pin == nil && column == focusedColumn
    }
}

private func tilesFrom(_ strip: JSONValue?) -> [IndicatorTile] {
    guard let strip else { return [] }
    let focusedColumn = strip["focused_column"]?.int ?? 0
    let focus = strip["focus"]?.string
    var tiles: [IndicatorTile] = []
    if let pin = strip["pinned_left"], pin.null == false {
        tiles.append(tile(pin, focused: tileIsFocused(focus: focus, focusedColumn: focusedColumn, pin: .left, column: -1), pin: .left, column: -1))
    }
    for column in strip["columns"]?.array ?? [] {
        let index = column["index"]?.int ?? tiles.count
        tiles.append(tile(column, focused: tileIsFocused(focus: focus, focusedColumn: focusedColumn, pin: nil, column: index), pin: nil, column: index))
    }
    if let pin = strip["pinned_right"], pin.null == false {
        tiles.append(tile(pin, focused: tileIsFocused(focus: focus, focusedColumn: focusedColumn, pin: .right, column: -2), pin: .right, column: -2))
    }
    return tiles
}

private func tile(_ column: JSONValue, focused: Bool, pin: PinEdge?, column index: Int) -> IndicatorTile {
    let windows = column["windows"]?.array ?? column["column"]?["windows"]?.array ?? []
    let lead = windows.first
    return IndicatorTile(
        title: lead?["title"]?.string ?? "",
        bundleID: lead?["bundle_id"]?.string ?? "",
        pid: Int32(truncatingIfNeeded: lead?["pid"]?.int ?? 0),
        count: max(windows.count, 1),
        focused: focused,
        pin: pin,
        windowID: lead?["id"]?.uint ?? 0,
        columnIndex: index
    )
}

private func managedWindows(_ strip: JSONValue?) -> [OverlayWindow] {
    guard let strip else { return [] }
    let focused = strip["focused_window"]?.uint
    var windows: [OverlayWindow] = []
    func add(_ column: JSONValue?) {
        for window in column?["windows"]?.array ?? column?["column"]?["windows"]?.array ?? [] {
            guard let id = window["id"]?.uint, let frame = window["frame"]?.frame else { continue }
            windows.append(OverlayWindow(frame: frame, focused: focused == id, visible: window["visible"]?.bool ?? false))
        }
    }
    add(strip["pinned_left"])
    for column in strip["columns"]?.array ?? [] { add(column) }
    add(strip["pinned_right"])
    return windows
}

private func tabColumn(_ strip: JSONValue?) -> TabColumn? {
    guard let strip else { return nil }
    if let focus = strip["focus"]?.string, focus != "column" { return nil }
    let focused = strip["focused_column"]?.int ?? 0
    let columns = strip["columns"]?.array ?? []
    guard let column = columns.first(where: { $0["index"]?.int == focused }) ?? columns[safe: focused],
          column["mode"]?.string == "tabbed" else {
        return nil
    }
    let windows = column["windows"]?.array ?? []
    let active = column["active_tab"]?.int ?? 0
    let shown = windows[safe: active] ?? windows.first
    guard shown?["visible"]?.bool ?? true else { return nil }
    return TabColumn(
        titles: windows.map { $0["title"]?.string ?? "" },
        active: active,
        ids: windows.map { $0["id"]?.uint ?? 0 },
        frame: shown?["frame"]?.frame
    )
}

final class IndicatorPanel: NSPanel {
    private let tilesView = IndicatorTilesView()
    var onFocus: ((IndicatorTile) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 240, height: IndicatorArt.pad * 2 + IndicatorArt.tile),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        configureOverlay(level: chromeLevel)
        let glass = installChromeBackground(around: tilesView, radius: IndicatorArt.panelRadius, clear: true)
        tilesView.drawsOutline = !glass
        tilesView.onFocus = { [weak self] tile in
            self?.onFocus?(tile)
        }
        tilesView.onMove = { [weak self] from, to in
            self?.onMove?(from, to)
        }
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    var isInteracting: Bool { tilesView.isInteracting }

    func update(_ tiles: [IndicatorTile]) {
        tilesView.tiles = tiles
    }
}

final class TabBarPanel: NSPanel {
    private let strip = TabStripView()
    private var shown: TabColumn?
    var onTab: ((UInt64) -> Void)?

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 24),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        configureOverlay(level: chromeLevel)
        let radius = TabStripView.height / 2
        let glass = installChromeBackground(around: strip, radius: glassAvailable ? radius : 7)
        strip.capsule = glass
        strip.onSelect = { [weak self] index in
            guard let self, let ids = self.shown?.ids, ids.indices.contains(index) else { return }
            self.onTab?(ids[index])
        }
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    fileprivate func update(_ column: TabColumn) {
        var content = column
        content.frame = nil
        guard content != shown else { return }
        shown = content
        strip.set(titles: column.titles, active: column.active)
    }
}

private final class TabStripView: NSView {
    static let height: CGFloat = 24
    private var titles: [String] = []
    private var active = 0
    var onSelect: ((Int) -> Void)?
    /// Glass bars are capsules, so the selected tab becomes a concentric capsule too.
    var capsule = false {
        didSet { if capsule != oldValue { needsDisplay = true } }
    }

    override var isFlipped: Bool { true }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func set(titles: [String], active: Int) {
        self.titles = titles
        self.active = active
        needsDisplay = true
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
        guard !titles.isEmpty else { return }
        let width = bounds.width / CGFloat(titles.count)
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.lineBreakMode = .byTruncatingTail
        let accent = IndicatorArt.systemAccent
        for (index, title) in titles.enumerated() {
            let cell = NSRect(x: CGFloat(index) * width, y: 0, width: width, height: bounds.height).insetBy(dx: 2, dy: 2)
            let selected = index == active
            if selected {
                let radius = capsule ? cell.height / 2 : 5
                accent.setFill()
                NSBezierPath(roundedRect: cell, xRadius: radius, yRadius: radius).fill()
            } else if index > 0, index - 1 != active {
                NSColor.separatorColor.setFill()
                NSRect(x: cell.minX - 2.5, y: cell.minY + 5, width: 1, height: max(cell.height - 10, 0)).fill()
            }
            let font = NSFont.systemFont(ofSize: 11, weight: selected ? .semibold : .medium)
            let text = NSAttributedString(string: title.isEmpty ? "\(index + 1)" : title, attributes: [
                .font: font,
                .foregroundColor: selected ? NSColor.white : NSColor.secondaryLabelColor,
                .paragraphStyle: paragraph,
            ])
            let lineHeight = ceil(font.ascender - font.descender)
            let textRect = NSRect(x: cell.minX + 6, y: cell.midY - lineHeight / 2, width: max(cell.width - 12, 0), height: lineHeight)
            text.draw(with: textRect, options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])
        }
    }

    override func mouseDown(with event: NSEvent) {
        guard !titles.isEmpty, bounds.width > 0 else { return }
        let point = convert(event.locationInWindow, from: nil)
        let index = Int(point.x / (bounds.width / CGFloat(titles.count)))
        onSelect?(min(max(index, 0), titles.count - 1))
    }
}

private extension JSONValue {
    var objectMap: [String: JSONValue]? {
        if case .object(let value) = self { return value }
        return nil
    }

    var array: [JSONValue]? {
        if case .array(let value) = self { return value }
        return nil
    }

    var string: String? {
        if case .string(let value) = self { return value }
        return nil
    }

    var bool: Bool? {
        if case .bool(let value) = self { return value }
        return nil
    }

    var int: Int? {
        switch self {
        case .int(let value): return value
        case .double(let value): return value.isFinite ? Int(exactly: value.rounded(.towardZero)) : nil
        default: return nil
        }
    }

    var uint: UInt64? {
        switch self {
        case .int(let value): return value >= 0 ? UInt64(value) : nil
        case .double(let value): return value.isFinite && value >= 0 ? UInt64(exactly: value.rounded(.towardZero)) : nil
        default: return nil
        }
    }

    var null: Bool {
        if case .null = self { return true }
        return false
    }

    var frame: CGRect? {
        guard let array, array.count == 4,
              let x = array[0].double, let y = array[1].double,
              let width = array[2].double, let height = array[3].double else { return nil }
        return CGRect(x: x, y: y, width: width, height: height)
    }

    var double: Double? {
        switch self {
        case .double(let value): return value
        case .int(let value): return Double(value)
        default: return nil
        }
    }

    subscript(key: String) -> JSONValue? { objectMap?[key] }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
