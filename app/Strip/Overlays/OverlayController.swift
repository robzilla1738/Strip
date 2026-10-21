import AppKit

final class OverlayController {
    private let indicator = IndicatorPanel()
    private let tabs = TabBarPanel()
    private var hideTimer: Timer?
    private var tiles: [IndicatorTile] = []
    private var tabbed: TabColumn?
    private var showIndicator = true
    var onFocus: ((UInt64) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?
    var onTab: ((UInt64) -> Void)?

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
    }

    func setShowIndicator(_ enabled: Bool) {
        showIndicator = enabled
        if !enabled { indicator.orderOut(nil) }
    }

    func apply(event: WireLine) {
        guard event.type == "event" else { return }
        switch event.event {
        case "indicator":
            let show = event.data?["show"]?.bool == true
            let hold = event.data?["hold"]?.bool == true
            presentIndicator(show: show || hold, hold: hold)
        case "state":
            if let data = event.data {
                applyState(data)
            }
        default:
            break
        }
    }

    func applyState(_ data: JSONValue) {
        let display = data["displays"]?.array?.first
        tiles = tilesFrom(display)
        tabbed = tabColumn(display)
        indicator.update(tiles)
        if let tabbed {
            tabs.update(tabbed)
            placeTabs(tabbed)
            tabs.orderFrontRegardless()
        } else {
            tabs.orderOut(nil)
        }
    }

    private func presentIndicator(show: Bool, hold: Bool) {
        hideTimer?.invalidate()
        guard showIndicator, show, !tiles.isEmpty else {
            fadeOut()
            return
        }
        placeIndicator()
        indicator.update(tiles)
        indicator.alphaValue = 1
        indicator.orderFrontRegardless()
        if !hold {
            hideTimer = Timer.scheduledTimer(withTimeInterval: 1.2, repeats: false) { [weak self] _ in
                self?.fadeOut()
            }
        }
    }

    private func fadeOut() {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.25
            indicator.animator().alphaValue = 0
        } completionHandler: { [indicator] in
            if indicator.alphaValue == 0 { indicator.orderOut(nil) }
        }
    }

    private func placeIndicator() {
        let size = IndicatorArt.size(count: tiles.count)
        guard let screen = NSScreen.screens.first,
              let visible = visibleFrame(of: screen) else { return }
        let primary = NSScreen.screens.first { $0.frame.origin == .zero }?.frame.height ?? screen.frame.height
        let x = visible.x + (visible.width - size.width) / 2
        let y = primary - (visible.y + visible.height) + 12
        indicator.setFrame(NSRect(x: x, y: y, width: size.width, height: size.height), display: true)
    }

    private func placeTabs(_ column: TabColumn) {
        guard let frame = column.frame else { return }
        let primary = NSScreen.screens.first { $0.frame.origin == .zero }?.frame.height ?? NSScreen.screens.first?.frame.height ?? 0
        let y = primary - (frame.y + 24)
        tabs.setFrame(NSRect(x: frame.x, y: y, width: max(frame.width, 1), height: 24), display: true)
    }

    private func visibleFrame(of screen: NSScreen) -> (x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat)? {
        let frame = screen.visibleFrame
        let primary = NSScreen.screens.first { $0.frame.origin == .zero }?.frame.height ?? screen.frame.height
        let axY = primary - frame.origin.y - frame.height
        return (frame.origin.x, axY, frame.width, frame.height)
    }
}

private struct TabColumn {
    var titles: [String]
    var active: Int
    var ids: [UInt64]
    var frame: (x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat)?
}

private func tilesFrom(_ display: JSONValue?) -> [IndicatorTile] {
    guard let display else { return [] }
    let strip = display["strips"]?.array?.first
    let focused = strip?["focused_column"]?.int ?? 0
    var tiles: [IndicatorTile] = []
    if let pin = strip?["pinned_left"], pin.null == false {
        tiles.append(tile(pin, focused: false, pin: .left, column: -1))
    }
    for column in strip?["columns"]?.array ?? [] {
        let index = column["index"]?.int ?? tiles.count
        tiles.append(tile(column, focused: index == focused, pin: nil, column: index))
    }
    if let pin = strip?["pinned_right"], pin.null == false {
        tiles.append(tile(pin, focused: false, pin: .right, column: -2))
    }
    return tiles
}

private func tile(_ column: JSONValue, focused: Bool, pin: PinEdge?, column index: Int) -> IndicatorTile {
    let windows = column["windows"]?.array ?? column["column"]?["windows"]?.array ?? []
    let title = windows.first?["title"]?.string ?? ""
    let id = windows.first?["id"]?.uint ?? 0
    return IndicatorTile(title: title, count: max(windows.count, 1), focused: focused, pin: pin, windowID: id, columnIndex: index)
}

private func tabColumn(_ display: JSONValue?) -> TabColumn? {
    let strip = display?["strips"]?.array?.first
    let focused = strip?["focused_column"]?.int ?? 0
    guard let column = strip?["columns"]?.array?.first(where: { $0["index"]?.int == focused }) ?? strip?["columns"]?.array?[safe: focused] else {
        return nil
    }
    guard column["mode"]?.string == "tabbed" else { return nil }
    let windows = column["windows"]?.array ?? []
    let frame = column["windows"]?.array?.first?["frame"]?.frame
    return TabColumn(
        titles: windows.map { $0["title"]?.string ?? "" },
        active: column["active_tab"]?.int ?? 0,
        ids: windows.map { $0["id"]?.uint ?? 0 },
        frame: frame
    )
}

final class IndicatorPanel: NSPanel {
    private let tilesView = IndicatorTilesView()

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 240, height: 56),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        hidesOnDeactivate = false
        let effect = NSVisualEffectView()
        effect.material = .hudWindow
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = 12
        effect.autoresizingMask = [.width, .height]
        tilesView.frame = effect.bounds
        tilesView.autoresizingMask = [.width, .height]
        effect.addSubview(tilesView)
        contentView = effect
    }

    func update(_ tiles: [IndicatorTile]) {
        tilesView.tiles = tiles
        tilesView.onFocus = { [weak self] tile in
            self?.onFocus?(tile)
        }
        tilesView.onMove = { [weak self] from, to in
            self?.onMove?(from, to)
        }
    }

    var onFocus: ((IndicatorTile) -> Void)?
    var onMove: ((_ from: Int, _ to: Int) -> Void)?
}

final class TabBarPanel: NSPanel {
    private let row = NSStackView()

    init() {
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 320, height: 24),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hidesOnDeactivate = false
        let effect = NSVisualEffectView()
        effect.material = .hudWindow
        effect.blendingMode = .behindWindow
        effect.state = .active
        row.orientation = .horizontal
        row.distribution = .fillEqually
        row.frame = effect.bounds
        row.autoresizingMask = [.width, .height]
        effect.addSubview(row)
        contentView = PassView(frame: .zero)
        contentView?.addSubview(effect)
        effect.autoresizingMask = [.width, .height]
        effect.frame = contentView?.bounds ?? .zero
    }

    fileprivate func update(_ column: TabColumn) {
        row.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (index, title) in column.titles.enumerated() {
            let button = NSButton(title: title.isEmpty ? "\(index + 1)" : title, target: self, action: #selector(clicked(_:)))
            button.tag = index
            button.isBordered = false
            button.font = .systemFont(ofSize: 11, weight: index == column.active ? .semibold : .regular)
            button.identifier = NSUserInterfaceItemIdentifier("tab")
            row.addArrangedSubview(button)
        }
        ids = column.ids
    }

    private var ids: [UInt64] = []
    var onTab: ((UInt64) -> Void)?

    @objc private func clicked(_ sender: NSButton) {
        guard ids.indices.contains(sender.tag) else { return }
        onTab?(ids[sender.tag])
    }
}

private final class PassView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        return hit is NSButton ? hit : nil
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
        case .double(let value): return Int(value)
        default: return nil
        }
    }

    var uint: UInt64? {
        switch self {
        case .int(let value): return UInt64(value)
        case .double(let value): return UInt64(value)
        default: return nil
        }
    }

    var null: Bool {
        if case .null = self { return true }
        return false
    }

    var frame: (x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat)? {
        guard let array, array.count == 4,
              let x = array[0].double, let y = array[1].double,
              let width = array[2].double, let height = array[3].double else { return nil }
        return (CGFloat(x), CGFloat(y), CGFloat(width), CGFloat(height))
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
