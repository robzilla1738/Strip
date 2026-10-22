import AppKit
import CryptoKit
import Foundation
import IOKit

protocol SecretStore {
    func data(for account: String) -> Data?
    func set(_ data: Data, for account: String)
    func remove(_ account: String)
}

final class MemorySecretStore: SecretStore {
    private var values: [String: Data] = [:]

    func data(for account: String) -> Data? { values[account] }
    func set(_ data: Data, for account: String) { values[account] = data }
    func remove(_ account: String) { values[account] = nil }
}

final class KeychainSecretStore: SecretStore {
    let service: String

    init(service: String) { self.service = service }

    func data(for account: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess else { return nil }
        return item as? Data
    }

    func set(_ data: Data, for account: String) {
        remove(account)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    func remove(_ account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}

struct LicenseEnvironment {
    var appName: String
    var productIDs: Set<String>
    var trialDays: Int
    var machineName: String
    var baseURL: URL?
    var checkoutURL: String

    /// False in local/dev builds where the brand values (payment env,
    /// products, checkout) were never filled in. There is nothing to buy or
    /// activate against, so licensing must not gate the app.
    var configured: Bool { baseURL != nil }
}

enum LicensePhase: Equatable {
    case trial(daysLeft: Int)
    case licensed(maskedKey: String, productID: String)
    case unlicensed(message: String)
    case notConfigured

    var allowsManaging: Bool {
        if case .unlicensed = self { return false }
        return true
    }

    var statusText: String {
        switch self {
        case .trial(let days): return "Trial \(days) days"
        case .licensed: return "Licensed"
        case .unlicensed(let message):
            return message.isEmpty ? "License required" : message
        case .notConfigured: return "Not configured (dev build)"
        }
    }
}

private struct LicenseRecord: Codable {
    var licenseKey: String
    var instanceID: String
    var productID: String
    var activatedAt: TimeInterval
    var lastValidatedAt: TimeInterval
}

private struct TrialRecord: Codable {
    var startedAt: TimeInterval
}

final class LicenseController: ObservableObject {
    @Published private(set) var phase: LicensePhase = .unlicensed(message: "License required")
    var onManaging: (Bool) -> Void = { _ in }

    let environment: LicenseEnvironment
    private let store: SecretStore
    private let session: URLSession
    private var lastAllow = true
    private let licenseAccount = "license"
    private let trialAccount = "trial"

    init(environment: LicenseEnvironment, store: SecretStore, session: URLSession) {
        self.environment = environment
        self.store = store
        self.session = session
        if !environment.configured {
            phase = .notConfigured
        }
    }

    static func live() -> LicenseController {
        let brand = Brand.current
        let base: URL?
        if brand.dodoEnv.isEmpty || brand.dodoEnv.contains("{") {
            fputs("MISSING: {{DODO_ENV}}\n", stderr)
            base = nil
        } else {
            base = URL(string: "https://\(brand.dodoEnv).dodopayments.com")
        }
        let trial: Int
        if let days = brand.trialDays {
            trial = days
        } else {
            fputs("MISSING: {{TRIAL_DAYS}}\n", stderr)
            trial = 0
        }
        let checkout = brand.checkoutURL
        if checkout.isEmpty || checkout.contains("{") {
            fputs("MISSING: {{DODO_CHECKOUT_URL}}\n", stderr)
        }
        let environment = LicenseEnvironment(
            appName: brand.appName,
            productIDs: Set(brand.productIDs),
            trialDays: trial,
            machineName: MachineIdentity.name(appName: brand.appName),
            baseURL: base,
            checkoutURL: checkout
        )
        let service = brand.bundleID + ".license"
        return LicenseController(environment: environment, store: KeychainSecretStore(service: service), session: .shared)
    }

    func startSchedule() {
        Timer.scheduledTimer(withTimeInterval: 24 * 60 * 60, repeats: true) { [weak self] _ in
            DispatchQueue.global(qos: .utility).async { self?.validate(now: Date()) }
        }
    }

    func refresh(now: Date) {
        guard environment.configured else {
            setPhase(.notConfigured)
            return
        }
        if loadLicense() != nil {
            validate(now: now)
        } else {
            applyTrial(now: now)
        }
    }

    @discardableResult
    func activate(_ key: String, now: Date) -> String? {
        let trimmed = key.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "invalid key" }
        guard let response = post("/licenses/activate", ["license_key": trimmed, "name": environment.machineName]) else {
            return "network error"
        }
        if let message = message(for: response.status) { return message }
        guard response.status == 201, let json = jsonObject(response.data) else { return "invalid key" }
        let instance = json["id"] as? String ?? ""
        let product = (json["product"] as? [String: Any])?["product_id"] as? String ?? ""
        guard environment.productIDs.contains(product) else { return "unknown product" }
        let record = LicenseRecord(
            licenseKey: trimmed,
            instanceID: instance,
            productID: product,
            activatedAt: now.timeIntervalSince1970,
            lastValidatedAt: now.timeIntervalSince1970
        )
        saveLicense(record)
        setPhase(.licensed(maskedKey: mask(trimmed), productID: product))
        return nil
    }

    func validate(now: Date) {
        guard var record = loadLicense() else {
            applyTrial(now: now)
            return
        }
        guard let response = post("/licenses/validate", [
            "license_key": record.licenseKey,
            "license_key_instance_id": record.instanceID,
        ]) else {
            let age = now.timeIntervalSince1970 - record.lastValidatedAt
            if age > 30 * 86_400 {
                setPhase(.unlicensed(message: "License required"))
            } else {
                setPhase(.licensed(maskedKey: mask(record.licenseKey), productID: record.productID))
            }
            return
        }
        let valid = (jsonObject(response.data)?["valid"] as? Bool) ?? false
        if response.status != 200 || !valid {
            store.remove(licenseAccount)
            setPhase(.unlicensed(message: "License required"))
            return
        }
        record.lastValidatedAt = now.timeIntervalSince1970
        saveLicense(record)
        setPhase(.licensed(maskedKey: mask(record.licenseKey), productID: record.productID))
    }

    @discardableResult
    func deactivate(now: Date) -> String? {
        guard let record = loadLicense() else {
            applyTrial(now: now)
            return nil
        }
        guard let response = post("/licenses/deactivate", [
            "license_key": record.licenseKey,
            "license_key_instance_id": record.instanceID,
        ]) else {
            return "network error"
        }
        guard (200..<300).contains(response.status) else {
            return message(for: response.status) ?? "network error"
        }
        store.remove(licenseAccount)
        applyTrial(now: now)
        return nil
    }

    func openCheckout() {
        let checkout = environment.checkoutURL
        guard checkout.hasPrefix("https://"), !checkout.contains("{"), let url = URL(string: checkout) else {
            fputs("MISSING: {{DODO_CHECKOUT_URL}}\n", stderr)
            return
        }
        NSWorkspace.shared.open(url)
    }

    private func applyTrial(now: Date) {
        guard environment.configured else {
            setPhase(.notConfigured)
            return
        }
        guard environment.trialDays > 0 else {
            setPhase(.unlicensed(message: "License required"))
            return
        }
        let started: TimeInterval
        if let record = loadTrial() {
            started = record.startedAt
        } else {
            started = now.timeIntervalSince1970
            saveTrial(TrialRecord(startedAt: started))
        }
        let elapsed = Int((now.timeIntervalSince1970 - started) / 86_400)
        let left = environment.trialDays - elapsed
        if left > 0 {
            setPhase(.trial(daysLeft: left))
        } else {
            setPhase(.unlicensed(message: "License required"))
        }
    }

    private func setPhase(_ phase: LicensePhase) {
        let apply = {
            self.phase = phase
            if phase.allowsManaging != self.lastAllow {
                self.lastAllow = phase.allowsManaging
                self.onManaging(phase.allowsManaging)
            }
        }
        if Thread.isMainThread {
            apply()
        } else {
            DispatchQueue.main.sync(execute: apply)
        }
    }

    private func message(for status: Int) -> String? {
        switch status {
        case 403: return "key not active"
        case 404: return "invalid key"
        case 422: return "activation limit reached — deactivate another Mac"
        default: return nil
        }
    }

    private func post(_ path: String, _ body: [String: String]) -> (status: Int, data: Data)? {
        guard let base = environment.baseURL else {
            fputs("MISSING: {{DODO_ENV}}\n", stderr)
            return nil
        }
        let root = base.absoluteString.hasSuffix("/") ? base.absoluteString : base.absoluteString + "/"
        guard let url = URL(string: root + path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))) else { return nil }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: body)
        let gate = DispatchSemaphore(value: 0)
        var result: (Int, Data)?
        let task = session.dataTask(with: request) { data, response, _ in
            if let http = response as? HTTPURLResponse {
                result = (http.statusCode, data ?? Data())
            }
            gate.signal()
        }
        task.resume()
        _ = gate.wait(timeout: .now() + 30)
        return result.map { (status: $0.0, data: $0.1) }
    }

    private func loadLicense() -> LicenseRecord? { decode(licenseAccount) }
    private func saveLicense(_ record: LicenseRecord) { encode(record, licenseAccount) }
    private func loadTrial() -> TrialRecord? { decode(trialAccount) }
    private func saveTrial(_ record: TrialRecord) { encode(record, trialAccount) }

    private func decode<T: Decodable>(_ account: String) -> T? {
        guard let data = store.data(for: account) else { return nil }
        return try? JSONDecoder().decode(T.self, from: data)
    }

    private func encode<T: Encodable>(_ value: T, _ account: String) {
        guard let data = try? JSONEncoder().encode(value) else { return }
        store.set(data, for: account)
    }

    private func jsonObject(_ data: Data) -> [String: Any]? {
        try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    private func mask(_ key: String) -> String {
        guard key.count > 4 else { return String(repeating: "•", count: key.count) }
        return String(repeating: "•", count: key.count - 4) + key.suffix(4)
    }
}

enum MachineIdentity {
    static func name(appName: String) -> String {
        let uuid = platformUUID() ?? "unknown"
        let digest = SHA256.hash(data: Data(uuid.utf8))
        let hex = digest.prefix(6).map { String(format: "%02x", $0) }.joined()
        return "\(appName)-\(hex)"
    }

    private static func platformUUID() -> String? {
        let service = IOServiceGetMatchingService(kIOMainPortDefault, IOServiceMatching("IOPlatformExpertDevice"))
        guard service != 0 else { return nil }
        defer { IOObjectRelease(service) }
        let value = IORegistryEntryCreateCFProperty(service, "IOPlatformUUID" as CFString, kCFAllocatorDefault, 0)
        return value?.takeRetainedValue() as? String
    }
}

final class LicenseAPIStub: URLProtocol {
    static var status = 200
    static var payload: Any = [:]
    static var fail = false
    static var lastBody: [String: String] = [:]

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        if let body = Self.requestBody(request),
           let object = try? JSONSerialization.jsonObject(with: body) as? [String: String] {
            Self.lastBody = object
        }
        if Self.fail {
            client?.urlProtocol(self, didFailWithError: URLError(.notConnectedToInternet))
            return
        }
        let data = (try? JSONSerialization.data(withJSONObject: Self.payload)) ?? Data()
        let response = HTTPURLResponse(url: request.url!, statusCode: Self.status, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func requestBody(_ request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: 1024)
        defer { buffer.deallocate() }
        while stream.hasBytesAvailable {
            let count = stream.read(buffer, maxLength: 1024)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}
