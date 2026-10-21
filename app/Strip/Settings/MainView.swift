import SwiftUI

struct MainView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if let settings = model.settings {
            SettingsRoot(model: model, settings: settings)
        } else {
            ProgressView()
                .frame(minWidth: 420, minHeight: 220)
        }
    }
}
