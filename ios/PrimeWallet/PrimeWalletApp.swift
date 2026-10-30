import SwiftUI

/// Prime Wallet: a watch-only Fedimint wallet whose spend keys Passport Prime holds.
@main
struct PrimeWalletApp: App {
    @StateObject private var model = WalletModel()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(model)
                .environmentObject(model.bridge)
        }
    }
}
