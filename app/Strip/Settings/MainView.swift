import SwiftUI

struct MainView: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Form {
            LabeledContent("Status", value: model.statusLine)
            LabeledContent("Accessibility", value: model.accessibilityLine)
        }
        .formStyle(.grouped)
        .frame(minWidth: 420, minHeight: 220)
        .navigationTitle(model.appName)
    }
}
