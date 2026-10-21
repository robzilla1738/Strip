import Foundation

struct URLCommand: Equatable {
    var action: String
    var args: JSONValue

    static func parse(_ url: URL, scheme: String) -> URLCommand? {
        guard let found = url.scheme, found.caseInsensitiveCompare(scheme) == .orderedSame else { return nil }
        guard let action = url.host, !action.isEmpty else { return nil }
        var fields: [String: JSONValue] = [:]
        if let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
           let raw = items.first(where: { $0.name == "n" })?.value,
           let number = Int(raw) {
            fields["n"] = .int(number)
        }
        return URLCommand(action: action, args: .object(fields))
    }
}
