import Foundation

struct Brand {
    var appName: String
    var bundleID: String
    var urlScheme: String
    var supportEmail: String
    var website: String
    var trialDaysRaw: String
    var dodoEnv: String
    var productIDs: [String]
    var checkoutURL: String

    static var current: Brand {
        let bundle = Bundle.main
        let info = bundle.infoDictionary ?? [:]
        let name = (info["CFBundleDisplayName"] as? String) ?? (info["CFBundleName"] as? String) ?? ""
        let types = info["CFBundleURLTypes"] as? [[String: Any]]
        let scheme = (types?.first?["CFBundleURLSchemes"] as? [String])?.first ?? ""
        let ids = ["StripProductID1", "StripProductID3", "StripProductID5", "StripProductID20"].compactMap {
            info[$0] as? String
        }
        return Brand(
            appName: name,
            bundleID: bundle.bundleIdentifier ?? "",
            urlScheme: scheme,
            supportEmail: info["StripSupportEmail"] as? String ?? "",
            website: info["StripWebsite"] as? String ?? "",
            trialDaysRaw: info["StripTrialDays"] as? String ?? "",
            dodoEnv: info["StripDodoEnv"] as? String ?? "",
            productIDs: ids.filter { !$0.isEmpty && !$0.contains("{") },
            checkoutURL: info["StripCheckoutURL"] as? String ?? ""
        )
    }

    var trialDays: Int? {
        guard !trialDaysRaw.isEmpty, !trialDaysRaw.contains("{"), let days = Int(trialDaysRaw) else {
            return nil
        }
        return days
    }
}
