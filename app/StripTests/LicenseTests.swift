import XCTest

final class LicenseTests: XCTestCase {
    private let product = "prod_seat"
    private let key = "ABCD-EFGH-IJKL"

    override func tearDown() {
        LicenseAPIStub.fail = false
        LicenseAPIStub.status = 200
        LicenseAPIStub.payload = [:]
        LicenseAPIStub.lastBody = [:]
        super.tearDown()
    }

    func testActivate201StoresALicense() {
        let (controller, store) = make()
        LicenseAPIStub.status = 201
        LicenseAPIStub.payload = ["id": "lki_1", "product": ["product_id": product]]
        XCTAssertNil(controller.activate(key, now: date(0)))
        XCTAssertEqual(LicenseAPIStub.lastBody["license_key"], key)
        XCTAssertEqual(LicenseAPIStub.lastBody["name"], "Example-0123456789ab")
        XCTAssertEqual(controller.phase, .licensed(maskedKey: masked(key), productID: product))
        XCTAssertNotNil(store.data(for: "license"))
    }

    func testActivate422IsTheActivationLimit() {
        let (controller, store) = make()
        LicenseAPIStub.status = 422
        LicenseAPIStub.payload = [:]
        XCTAssertEqual(controller.activate(key, now: date(0)), "activation limit reached — deactivate another Mac")
        XCTAssertEqual(controller.phase, .unlicensed(message: "License required"))
        XCTAssertNil(store.data(for: "license"))
    }

    func testActivate404IsAnInvalidKey() {
        let (controller, _) = make()
        LicenseAPIStub.status = 404
        XCTAssertEqual(controller.activate(key, now: date(0)), "invalid key")
    }

    func testActivate403IsNotActive() {
        let (controller, _) = make()
        LicenseAPIStub.status = 403
        XCTAssertEqual(controller.activate(key, now: date(0)), "key not active")
    }

    func testValidateFalseClearsTheLicenseImmediately() {
        let (controller, store) = make()
        LicenseAPIStub.status = 201
        LicenseAPIStub.payload = ["id": "lki_1", "product": ["product_id": product]]
        XCTAssertNil(controller.activate(key, now: date(0)))
        LicenseAPIStub.status = 200
        LicenseAPIStub.payload = ["valid": false]
        controller.validate(now: date(1))
        XCTAssertEqual(controller.phase, .unlicensed(message: "License required"))
        XCTAssertNil(store.data(for: "license"))
    }

    func testOfflineFor13DaysStaysLicensed() {
        let (controller, _) = make()
        LicenseAPIStub.status = 201
        LicenseAPIStub.payload = ["id": "lki_1", "product": ["product_id": product]]
        XCTAssertNil(controller.activate(key, now: date(0)))
        LicenseAPIStub.fail = true
        controller.validate(now: date(13))
        XCTAssertEqual(controller.phase, .licensed(maskedKey: masked(key), productID: product))
    }

    func testOfflineFor31DaysBecomesUnlicensed() {
        let (controller, _) = make()
        LicenseAPIStub.status = 201
        LicenseAPIStub.payload = ["id": "lki_1", "product": ["product_id": product]]
        XCTAssertNil(controller.activate(key, now: date(0)))
        LicenseAPIStub.fail = true
        controller.validate(now: date(31))
        XCTAssertEqual(controller.phase, .unlicensed(message: "License required"))
    }

    func testDeactivateClearsTheStore() {
        let (controller, store) = make()
        LicenseAPIStub.status = 201
        LicenseAPIStub.payload = ["id": "lki_1", "product": ["product_id": product]]
        XCTAssertNil(controller.activate(key, now: date(0)))
        LicenseAPIStub.status = 200
        LicenseAPIStub.payload = [:]
        XCTAssertNil(controller.deactivate(now: date(1)))
        XCTAssertNil(store.data(for: "license"))
        XCTAssertEqual(controller.phase, .unlicensed(message: "License required"))
    }

    func testUnconfiguredBuildAllowsManaging() {
        let store = MemorySecretStore()
        let environment = LicenseEnvironment(
            appName: "Example",
            productIDs: [],
            trialDays: 0,
            machineName: "Example-0123456789ab",
            baseURL: nil,
            checkoutURL: ""
        )
        let controller = LicenseController(environment: environment, store: store, session: .shared)
        var decisions: [Bool] = []
        controller.onManaging = { decisions.append($0) }
        XCTAssertEqual(controller.phase, .notConfigured)
        controller.refresh(now: date(0))
        XCTAssertEqual(controller.phase, .notConfigured)
        XCTAssertTrue(controller.phase.allowsManaging)
        XCTAssertFalse(decisions.contains(false))
    }

    private func make() -> (LicenseController, MemorySecretStore) {
        let store = MemorySecretStore()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [LicenseAPIStub.self]
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1
        let environment = LicenseEnvironment(
            appName: "Example",
            productIDs: [product],
            trialDays: 0,
            machineName: "Example-0123456789ab",
            baseURL: URL(string: "https://test.dodopayments.com"),
            checkoutURL: ""
        )
        let session = URLSession(configuration: configuration, delegate: nil, delegateQueue: queue)
        let controller = LicenseController(environment: environment, store: store, session: session)
        return (controller, store)
    }

    private func date(_ days: Int) -> Date {
        Date(timeIntervalSince1970: Double(days) * 86_400)
    }

    private func masked(_ key: String) -> String {
        String(repeating: "•", count: key.count - 4) + key.suffix(4)
    }
}
