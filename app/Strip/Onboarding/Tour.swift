import Foundation

final class TourProgress: ObservableObject {
    @Published private(set) var index = 0

    static let titles = [
        "Focus left or right",
        "Move a column",
        "Cycle width",
        "Stack a window",
        "Swipe the strip",
    ]

    var finished: Bool { index >= Self.titles.count }

    func receive(_ action: String) {
        guard index < Self.titles.count else { return }
        let advance: Bool
        switch index {
        case 0: advance = action == "focus_left" || action == "focus_right"
        case 1: advance = action.hasPrefix("move_")
        case 2: advance = action == "cycle_width"
        case 3: advance = action.hasPrefix("stack_")
        case 4: advance = action == "swipe"
        default: advance = false
        }
        if advance { index += 1 }
    }
}

enum Onboarding {
    static let defaultsKey = "hasCompletedOnboarding"

    static var shouldShow: Bool {
        !UserDefaults.standard.bool(forKey: defaultsKey)
    }

    static func complete() {
        UserDefaults.standard.set(true, forKey: defaultsKey)
    }
}
