import Foundation

/// The app's state around the Rust wallet.
@MainActor
final class WalletModel: ObservableObject {
    let bridge = PrimeBridge()

    @Published private(set) var wallet: Wallet?
    @Published private(set) var federation: FederationSummary?
    @Published private(set) var balanceMsat: UInt64 = 0
    @Published private(set) var activity: [Activity] = []
    @Published private(set) var pendingSends: [PendingSend] = []
    /// What the wallet is doing, while it does it.
    @Published var busy: String?
    /// The last failure, until the person dismisses it.
    @Published var error: String?
    @Published private(set) var openError: String?

    /// Opens the wallet kept in the app's Application Support folder.
    func open() async {
        guard wallet == nil else { return }
        do {
            let directory = try FileManager.default
                .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                .appendingPathComponent("wallet", isDirectory: true)
            let wallet = try await Wallet.open(dir: directory.path, link: BridgeLink(bridge: bridge))
            self.wallet = wallet
            federation = await wallet.federation()
            await refresh()
        } catch {
            openError = error.shown
        }
    }

    func refresh() async {
        guard let wallet, federation != nil else { return }
        do {
            balanceMsat = try await wallet.balanceMsat()
            activity = try await wallet.activity()
            pendingSends = try await wallet.pendingSends()
        } catch {
            self.error = error.shown
        }
    }

    /// Runs one wallet call behind a progress note, reports a failure, and refreshes after.
    @discardableResult
    func run<T>(_ note: String, _ call: (Wallet) async throws -> T) async -> T? {
        guard let wallet else { return nil }
        busy = note
        defer { busy = nil }
        do {
            let result = try await call(wallet)
            await refresh()
            return result
        } catch {
            self.error = error.shown
            await refresh()
            return nil
        }
    }

    /// Leaves the federation. Prime keeps it, so joining again recovers the balance.
    func leave() async {
        guard let wallet else { return }
        busy = "Leaving the federation"
        defer { busy = nil }
        do {
            try await wallet.leave()
            federation = nil
            balanceMsat = 0
            activity = []
            pendingSends = []
        } catch {
            self.error = error.shown
        }
    }

    func join(_ inviteCode: String) async {
        let summary = await run("Joining the federation and pairing it with Prime") {
            try await $0.join(inviteCode: inviteCode)
        }
        if let summary {
            federation = summary
            await refresh()
        }
    }
}
