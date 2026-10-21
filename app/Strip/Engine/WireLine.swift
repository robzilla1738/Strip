import Foundation

enum JSONValue: Codable, Equatable {
    case null
    case bool(Bool)
    case int(Int)
    case double(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
            return
        }
        if let value = try? container.decode(Bool.self) {
            self = .bool(value)
            return
        }
        if let value = try? container.decode(Int.self) {
            self = .int(value)
            return
        }
        if let value = try? container.decode(Double.self) {
            self = .double(value)
            return
        }
        if let value = try? container.decode(String.self) {
            self = .string(value)
            return
        }
        if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
            return
        }
        if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
            return
        }
        throw DecodingError.dataCorruptedError(in: container, debugDescription: "unsupported json")
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case .bool(let value):
            try container.encode(value)
        case .int(let value):
            try container.encode(value)
        case .double(let value):
            try container.encode(value)
        case .string(let value):
            try container.encode(value)
        case .array(let value):
            try container.encode(value)
        case .object(let value):
            try container.encode(value)
        }
    }
}

struct PrivateFlags: Codable, Equatable {
    let skylight: Bool
    let axGetWindow: Bool

    enum CodingKeys: String, CodingKey {
        case skylight
        case axGetWindow = "ax_get_window"
    }
}

struct ErrorBody: Codable, Equatable {
    let code: String
    let message: String
}

struct WireLine: Codable, Equatable {
    var id: UInt64?
    var type: String
    var client: String?
    var protocolVersion: UInt32?
    var action: String?
    var args: JSONValue?
    var what: String?
    var events: [String]?
    var enabled: Bool?
    var engineVersion: String?
    var axTrusted: Bool?
    var privateAPI: PrivateFlags?
    var ok: Bool?
    var data: JSONValue?
    var error: ErrorBody?
    var event: String?

    enum CodingKeys: String, CodingKey {
        case id
        case type
        case client
        case action
        case args
        case what
        case events
        case enabled
        case ok
        case data
        case error
        case event
        case protocolVersion = "protocol"
        case engineVersion = "engine_version"
        case axTrusted = "ax_trusted"
        case privateAPI = "private_api"
    }
}

struct HelloRequest: Encodable {
    let id: UInt64
    let type: String
    let client: String
    let protocolVersion: UInt32

    enum CodingKeys: String, CodingKey {
        case id
        case type
        case client
        case protocolVersion = "protocol"
    }
}

struct CommandRequest: Encodable {
    let id: UInt64
    let type: String
    let action: String
    let args: JSONValue
}

struct QueryRequest: Encodable {
    let id: UInt64
    let type: String
    let what: String
}

struct IdRequest: Encodable {
    let id: UInt64
    let type: String
}

struct ManagingRequest: Encodable {
    let id: UInt64
    let type: String
    let enabled: Bool
}

struct SubscribeRequest: Encodable {
    let id: UInt64
    let type: String
    let events: [String]
}

extension WireLine {
    var tourAction: String? {
        guard event == "tour", case let .object(fields) = data, case let .string(action) = fields["action"] else {
            return nil
        }
        return action
    }
}

extension JSONValue {
    /// Mirrors the Codable decoder: integral numbers become `.int`, the rest `.double`.
    init?(foundation raw: Any) {
        switch raw {
        case is NSNull:
            self = .null
        case let value as String:
            self = .string(value)
        case let number as NSNumber:
            if CFGetTypeID(number) == CFBooleanGetTypeID() {
                self = .bool(number.boolValue)
            } else if let value = exactInt(number) {
                self = .int(value)
            } else {
                self = .double(number.doubleValue)
            }
        case let values as [Any]:
            var items: [JSONValue] = []
            items.reserveCapacity(values.count)
            for value in values {
                guard let item = JSONValue(foundation: value) else { return nil }
                items.append(item)
            }
            self = .array(items)
        case let fields as [String: Any]:
            var object: [String: JSONValue] = [:]
            object.reserveCapacity(fields.count)
            for (key, value) in fields {
                guard let item = JSONValue(foundation: value) else { return nil }
                object[key] = item
            }
            self = .object(object)
        default:
            return nil
        }
    }
}

private func exactInt(_ number: NSNumber) -> Int? {
    guard CFNumberIsFloatType(number) else { return number.intValue }
    let value = number.doubleValue
    guard value.rounded() == value, value >= -9.2e18, value <= 9.2e18 else { return nil }
    return Int(value)
}

extension WireLine {
    /// JSONSerialization-backed equivalent of `JSONDecoder().decode(WireLine.self, from:)`.
    /// Returns nil wherever the Codable path would throw.
    static func parse(_ data: Data) -> WireLine? {
        guard let raw = try? JSONSerialization.jsonObject(with: data),
              let fields = raw as? [String: Any] else { return nil }
        return try? WireLine(fields: fields)
    }

    private struct Mismatch: Error {}

    private init(fields: [String: Any]) throws {
        func field<T>(_ key: String, _ convert: (Any) -> T?) throws -> T? {
            guard let raw = fields[key], !(raw is NSNull) else { return nil }
            guard let value = convert(raw) else { throw Mismatch() }
            return value
        }
        guard let type = fields["type"] as? String else { throw Mismatch() }
        self.type = type
        id = try field("id", unsigned)
        client = try field("client") { $0 as? String }
        protocolVersion = try field("protocol") { unsigned($0).flatMap { UInt32(exactly: $0) } }
        action = try field("action") { $0 as? String }
        args = try field("args") { JSONValue(foundation: $0) }
        what = try field("what") { $0 as? String }
        events = try field("events") { raw in
            guard let values = raw as? [Any] else { return nil }
            let strings = values.compactMap { $0 as? String }
            return strings.count == values.count ? strings : nil
        }
        enabled = try field("enabled", boolean)
        engineVersion = try field("engine_version") { $0 as? String }
        axTrusted = try field("ax_trusted", boolean)
        privateAPI = try field("private_api") { raw in
            guard let flags = raw as? [String: Any],
                  let skylight = flags["skylight"].flatMap(boolean),
                  let axGetWindow = flags["ax_get_window"].flatMap(boolean) else { return nil }
            return PrivateFlags(skylight: skylight, axGetWindow: axGetWindow)
        }
        ok = try field("ok", boolean)
        data = try field("data") { JSONValue(foundation: $0) }
        error = try field("error") { raw in
            guard let body = raw as? [String: Any],
                  let code = body["code"] as? String,
                  let message = body["message"] as? String else { return nil }
            return ErrorBody(code: code, message: message)
        }
        event = try field("event") { $0 as? String }
    }
}

private func boolean(_ raw: Any) -> Bool? {
    guard let number = raw as? NSNumber, CFGetTypeID(number) == CFBooleanGetTypeID() else { return nil }
    return number.boolValue
}

private func unsigned(_ raw: Any) -> UInt64? {
    guard let number = raw as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID() else { return nil }
    if CFNumberIsFloatType(number) {
        return UInt64(exactly: number.doubleValue)
    }
    return number.int64Value >= 0 ? number.uint64Value : nil
}
