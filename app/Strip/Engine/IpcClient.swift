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
    private let queue = DispatchQueue(label: "ipc.client", qos: .userInteractive)
    /// Guards `connection`, `token`, `nextID` and `waiters`.
    private let lock = NSLock()
    private let turn = NSLock()
    private var connection: NWConnection?
    /// Only touched on `queue`.
    private var buffer = Data()
    private var nextID: UInt64 = 1
    private var token = 0
    private var waiters: [UInt64: (Result<WireLine, Error>) -> Void] = [:]
    /// Called on the client queue for every event line.
    var onEvent: ((WireLine) -> Void)?
    /// Called on the client queue when the engine drops the stream. Not called after `close()`.
    var onClose: ((Error) -> Void)?

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

    func subscribe(_ events: [String]) throws -> WireLine {
        let id = allocate()
        return try roundTrip(SubscribeRequest(id: id, type: "subscribe", events: events), id: id)
    }

    func shutdown() {
        let id = allocate()
        _ = try? roundTrip(IdRequest(id: id, type: "shutdown"), id: id, timeout: 0.6)
    }

    func close() {
        lock.lock()
        token += 1
        let current = connection
        connection = nil
        lock.unlock()
        current?.cancel()
        failAll(IpcFailure.disconnected("closed"))
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
        lock.lock()
        token += 1
        let token = token
        let previous = connection
        connection = nil
        lock.unlock()
        previous?.cancel()
        queue.sync { buffer.removeAll() }
        let ready = Box<String>()
        let conn = NWConnection(to: .unix(path: path), using: Self.unixParameters())
        conn.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready:
                ready.put("ready")
            case .waiting(let error):
                // A missing or refused socket parks the connection in `.waiting`; fail now so the caller can retry.
                ready.put("failed:\(error)")
            case .failed(let error):
                ready.put("failed:\(error)")
                self?.ended(token: token, error: error)
            case .cancelled:
                ready.put("failed:cancelled")
            default:
                break
            }
        }
        conn.start(queue: queue)
        guard let state = ready.wait(seconds: 2) else {
            conn.cancel()
            throw IpcFailure.timedOut
        }
        guard state == "ready" else {
            conn.cancel()
            throw IpcFailure.disconnected(state)
        }
        lock.lock()
        guard token == self.token else {
            lock.unlock()
            conn.cancel()
            throw IpcFailure.disconnected("superseded")
        }
        connection = conn
        lock.unlock()
        receive(on: conn, token: token)
    }

    private func roundTrip<T: Encodable>(_ body: T, id: UInt64, timeout: TimeInterval = 3) throws -> WireLine {
        turn.lock()
        defer { turn.unlock() }
        let payload = try JSONEncoder().encode(body) + Data([0x0A])
        let box = Box<Result<WireLine, Error>>()
        lock.lock()
        guard let connection else {
            lock.unlock()
            throw IpcFailure.disconnected("no connection")
        }
        waiters[id] = { box.put($0) }
        lock.unlock()
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

    private func isCurrent(_ token: Int) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return token == self.token
    }

    private func receive(on conn: NWConnection, token: Int) {
        conn.receive(minimumIncompleteLength: 1, maximumLength: 262_144) { [weak self] data, _, isComplete, error in
            guard let self, self.isCurrent(token) else { return }
            if let data, !data.isEmpty {
                self.consume(data)
            }
            if let error {
                self.ended(token: token, error: error)
                return
            }
            if isComplete {
                self.ended(token: token, error: IpcFailure.disconnected("closed"))
                return
            }
            self.receive(on: conn, token: token)
        }
    }

    private func ended(token: Int, error: Error) {
        lock.lock()
        guard token == self.token else {
            lock.unlock()
            return
        }
        self.token += 1
        let current = connection
        connection = nil
        lock.unlock()
        current?.cancel()
        failAll(error)
        onClose?(error)
    }

    private func consume(_ data: Data) {
        buffer.append(data)
        var start = buffer.startIndex
        while let newline = buffer[start...].firstIndex(of: 0x0A) {
            let chunk = buffer[start..<newline]
            start = buffer.index(after: newline)
            guard !chunk.isEmpty, let line = WireLine.parse(Data(chunk)) else {
                continue
            }
            if line.type == "event" {
                onEvent?(line)
            } else if let id = line.id {
                deliver(id: id, result: .success(line))
            }
        }
        buffer.removeSubrange(buffer.startIndex..<start)
        if buffer.count > 1 << 20 {
            buffer.removeAll()
            failAll(IpcFailure.badLine("line exceeds 1 MiB"))
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

/// One-shot value handoff: the first `put` wins and wakes the waiter.
private final class Box<T> {
    private let lock = NSLock()
    private let signal = DispatchSemaphore(value: 0)
    private var value: T?

    func put(_ value: T) {
        lock.lock()
        guard self.value == nil else {
            lock.unlock()
            return
        }
        self.value = value
        lock.unlock()
        signal.signal()
    }

    func wait(seconds: TimeInterval) -> T? {
        _ = signal.wait(timeout: .now() + seconds)
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}
