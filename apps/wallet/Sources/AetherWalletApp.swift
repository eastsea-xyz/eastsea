import SwiftUI

@main
struct AetherWalletApp: App {
    @StateObject private var model = WalletModel()

    var body: some Scene {
        WindowGroup("Aether Wallet") {
            ContentView().environmentObject(model)
        }
        #if os(macOS)
        .windowResizability(.contentSize)
        #endif
    }
}
