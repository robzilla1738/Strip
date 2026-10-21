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
