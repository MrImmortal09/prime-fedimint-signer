import SwiftUI

/// Ecash Wallet: a watch-only Fedimint wallet whose spend keys Passport Prime holds.
@main
struct PrimeWalletApp: App {
    @StateObject private var model = WalletModel()
#if canImport(fps_cashuFFI)
    /// The Cashu wallet, in a build with the core from scripts/ios-core-cashu.sh.
    @StateObject private var cashu = CashuModel()

    init() {
        // Before either wallet opens: aws-lc-rs as rustls's process default, the provider fedimint
        // installs, so a mint client made first cannot install ring for both.
        cashuInstallTls()
    }
#endif

    var body: some Scene {
        WindowGroup {
#if canImport(fps_cashuFFI)
            ZStack {
                RootView()
                    .environmentObject(model)
                    .environmentObject(model.bridge)
                // Above RootView, which keeps the Prime sheet: Cashu requests use the same bridge.
                CashuLayer()
                    .environmentObject(cashu)
                    .environmentObject(model)
                    .environmentObject(model.bridge)
            }
#else
            RootView()
                .environmentObject(model)
                .environmentObject(model.bridge)
#endif
        }
    }
}
