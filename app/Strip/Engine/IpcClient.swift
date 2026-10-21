import Foundation
import Network

enum IpcFailure: Error, CustomStringConvertible {
    case timedOut
    case disconnected(String)
    case badLine(String)

    var description: String {
        switch self {
        case .timedOut:
            return "timed out"
        case .disconnected(let message):
            return message
        case .badLine(let message):
            return message
        }
    }
}

/// Line-framed JSON over a Unix-domain `NWConnection`.
final class IpcClient {
    private let queue = DispatchQueue(label: "ipc.client")
    private let lock = NSLock()
    private let turn = NSLock()
    private var connection: NWConnection?
    private var buffer = Data()
    private var nextID: UInt64 = 1
    private var token = 0
    private var waiters: [UInt64: (Result<WireLine, Error>) -> Void] = [:]

    func open(socketPath: String, client: String) throws -> WireLine {
        try connect(socketPath)
        let id = allocate()
        return try roundTrip(HelloRequest(id: id, type: "hello", client: client, protocolVersion: 1), id: id)
    }

    func query(_ what: String) throws -> WireLine {
        let id = allocate()
        return try roundTrip(QueryRequest(id: id, type: "query", what: what), id: id)
    }

    func command(_ action: String, args: JSONValue = .object([:])) throws -> WireLine {
        let id = allocate()
        return try roundTrip(CommandRequest(id: id, type: "command", action: action, args: args), id: id)
    }

    func setManaging(_ enabled: Bool) throws -> WireLine {
        let id = allocate()
        return try roundTrip(ManagingRequest(id: id, type: "set_managing", enabled: enabled), id: id)
    }

    func shutdown() {
        let id = allocate()
        _ = try? roundTrip(IdRequest(id: id, type: "shutdown"), id: id, timeout: 0.6)
    }

    func close() {
        token += 1
        connection?.cancel()
        connection = nil
    }

    /// A nil transport fails to attach to a BSD Unix listener (`null protocol_parameters`).
    /// `NWParameters.tcp` on a `.unix` endpoint is a raw byte stream here: the engine's
    /// line-framed hello comes back with no extra header.
    private static func unixParameters() -> NWParameters {
        let tcp = NWProtocolTCP.Options()
        tcp.noDelay = true
        return NWParameters(tls: nil, tcp: tcp)
    }

    private func connect(_ path: String) throws {
        token += 1
        let token = token
        connection?.cancel()
        buffer.removeAll()
        let ready = Box<String>()
        let conn = NWConnection(to: .unix(path: path), using: Self.unixParameters())
        conn.stateUpdateHandler = { state in
            switch state {
            case .ready:
                ready.put("ready")
            case .failed(let error):
                ready.put("failed:\(error)")
            case .cancelled:
                ready.put("failed:cancelled")
            default:
                break
            }
        }
        connection = conn
        conn.start(queue: queue)
        guard let state = ready.wait(seconds: 2) else {
            throw IpcFailure.timedOut
        }
        guard state == "ready" else {
            throw IpcFailure.disconnected(state)
        }
        receive(token: token)
    }

    private func roundTrip<T: Encodable>(_ body: T, id: UInt64, timeout: TimeInterval = 3) throws -> WireLine {
        turn.lock()
        defer { turn.unlock() }
        let payload = try JSONEncoder().encode(body) + Data([0x0A])
        let box = Box<Result<WireLine, Error>>()
        lock.lock()
        waiters[id] = { box.put($0) }
        lock.unlock()
        guard let connection else {
            throw IpcFailure.disconnected("no connection")
        }
        connection.send(content: payload, completion: .contentProcessed { [weak self] error in
            if let error {
                self?.deliver(id: id, result: .failure(error))
            }
        })
        guard let result = box.wait(seconds: timeout) else {
            lock.lock()
            waiters.removeValue(forKey: id)
            lock.unlock()
            throw IpcFailure.timedOut
        }
        return try result.get()
    }

    private func allocate() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        nextID += 1
        return nextID
    }

    private func receive(token: Int) {
        connection?.receive(minimumIncompleteLength: 1, maximumLength: 65_536) { [weak self] data, _, isComplete, error in
            guard let self, token == self.token else { return }
            if let data, !data.isEmpty {
                self.consume(data)
            }
            if let error {
                self.failAll(error)
                return
            }
            if isComplete {
                self.failAll(IpcFailure.disconnected("closed"))
                return
            }
            self.receive(token: token)
        }
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        if buffer.count > 1 << 20 {
            failAll(IpcFailure.badLine("line exceeds 1 MiB"))
            buffer.removeAll()
            return
        }
        while let newline = buffer.firstIndex(of: 0x0A) {
            let chunk = buffer.prefix(upTo: newline)
            buffer.removeSubrange(...newline)
            guard !chunk.isEmpty, let line = try? JSONDecoder().decode(WireLine.self, from: Data(chunk)) else {
                continue
            }
            if let id = line.id {
                deliver(id: id, result: .success(line))
            }
        }
    }

    private func deliver(id: UInt64, result: Result<WireLine, Error>) {
        lock.lock()
        let waiter = waiters.removeValue(forKey: id)
        lock.unlock()
        waiter?(result)
    }

    private func failAll(_ error: Error) {
        lock.lock()
        let pending = waiters
        waiters.removeAll()
        lock.unlock()
        for waiter in pending.values {
            waiter(.failure(error))
        }
    }
}

private final class Box<T> {
    private let lock = NSLock()
    private var value: T?

    func put(_ value: T) {
        lock.lock()
        if self.value == nil {
            self.value = value
        }
        lock.unlock()
    }

    func wait(seconds: TimeInterval) -> T? {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            lock.lock()
            let current = value
            lock.unlock()
            if let current {
                return current
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
        lock.lock()
        let current = value
        lock.unlock()
        return current
    }
}
