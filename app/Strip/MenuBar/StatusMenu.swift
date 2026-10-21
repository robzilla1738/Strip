import SwiftUI

struct StatusMenu: View {
    @ObservedObject var model: AppModel

    var body: some View {
        Button(model.statusLine) {}
            .disabled(true)
        Divider()
        Button("Start") { model.startManaging() }
        Button("Stop") { model.stopManaging() }
        Button("Restart Engine") { model.restartEngine() }
        Divider()
        Button("Open \(model.appName)…") { model.openMain() }
            .disabled(model.appName.isEmpty)
        Button("Check for Updates…") { model.onCheckForUpdates() }
            .disabled(!model.canCheckForUpdates)
        Button("Install Command Line Tool…") { model.installCommandLineTool() }
        Divider()
        Menu("Help") {
            Button("Setup Guide") { model.openGuide() }
            Button("Reveal Engine Log") { model.revealLog() }
            Button("Reveal Config File") { model.revealConfig() }
        }
        Button("About") { model.openAbout() }
        Divider()
        Button("Quit") { model.quit() }
    }
}
