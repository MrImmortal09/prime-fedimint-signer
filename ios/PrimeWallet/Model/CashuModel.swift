#if canImport(fps_cashuFFI)
import Foundation

/// The app's state around the Rust Cashu wallet, beside the Fedimint `WalletModel`: its own
/// folder, the same Passport Prime and the same connection settings.
@MainActor
final class CashuModel: ObservableObject {
    @Published private(set) var wallet: CashuWallet?
    @Published private(set) var mints: [CashuMint] = []
    @Published private(set) var balanceSat: UInt64 = 0
    @Published private(set) var sends: [CashuSend] = []
    @Published private(set) var activity: [CashuActivity] = []
    /// What the wallet is doing, while it does it.
    @Published var busy: String?
    /// The last failure, until the person dismisses it.
    @Published var error: String?
    @Published private(set) var openError: String?
    private var opening = false
    private var refreshing = false
    private var refreshAgain = false

    /// Opens the Cashu wallet kept in Application Support/cashu/, a sibling of the Fedimint
    /// wallet's folder, which it never touches. Its requests to Prime go through `bridge`, the one
    /// the Fedimint wallet uses, so they show on the same Prime sheet.
    func open(bridge: PrimeBridge) async {
        guard wallet == nil, !opening else { return }
        opening = true
        defer { opening = false }
        openError = nil
        do {
            let directory = try FileManager.default
                .url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                .appendingPathComponent("cashu", isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            // The app installed it at launch; this installs nothing when a provider is set.
            cashuInstallTls()
            let wallet = try await CashuWallet.open(dir: directory.path, link: CashuBridgeLink(bridge: bridge))
            self.wallet = wallet
            applyConnection()
            await refresh()
        } catch {
            openError = error.cashuShown
        }
    }

    /// Where requests to Prime go, from the settings the Fedimint wallet reads
    /// (`WalletModel.applyConnection`): the app in the KeyOS simulator over the network when chosen
    /// and an address is set, QR codes on the shared Prime sheet otherwise. Applied again before
    /// every call, so a change in Settings counts at once.
    func applyConnection() {
        let defaults = UserDefaults.standard
        let simulator = defaults.string(forKey: "primeConnection") == "simulator"
        let address = defaults.string(forKey: "simulatorAddress")?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        wallet?.setSimulatorAddress(address: simulator && !address.isEmpty ? address : nil)
    }

    /// Reads the mints, balance, sends and activity again. A refresh asked for while one runs makes
    /// that one read once more, so what a call just changed always shows.
    func refresh(reportErrors: Bool = false) async {
        guard let wallet else { return }
        if refreshing {
            refreshAgain = true
            return
        }
        refreshing = true
        defer { refreshing = false }
        repeat {
            refreshAgain = false
            var failure: Error?
            do { mints = try await wallet.mints() } catch { failure = failure ?? error }
            do { balanceSat = try await wallet.balanceSat() } catch { failure = failure ?? error }
            do { sends = try await wallet.sends() } catch { failure = failure ?? error }
            do { activity = try await wallet.activity() } catch { failure = failure ?? error }
            if reportErrors, let failure { self.error = failure.cashuShown }
        } while refreshAgain
    }

    /// Runs one wallet call behind a progress note, reports a failure, and refreshes after.
    @discardableResult
    func run<T>(_ note: String, _ call: (CashuWallet) async throws -> T) async -> T? {
        guard let wallet, busy == nil else { return nil }
        applyConnection()
        busy = note
        defer { busy = nil }
        do {
            let result = try await call(wallet)
            await refresh()
            return result
        } catch {
            self.error = error.cashuShown
            await refresh()
            return nil
        }
    }

    /// Prime approves "Cashu: add mint" and answers with the mint's lock material.
    func addMint(url: String) async -> CashuMint? {
        await run("Adding the mint; approve on Prime") { try await $0.addMint(url: url) }
    }

    /// A mint quote: the invoice to pay. No Prime.
    func topUp(mintUrl: String, amountSat: UInt64) async -> CashuTopUp? {
        await run("Asking the mint for an invoice") { try await $0.topUp(mintUrl: mintUrl, amountSat: amountSat) }
    }

    /// Waits for the invoice to be paid, then mints into proofs locked to Prime. No Prime.
    func finishTopUp(quoteId: String) async -> UInt64? {
        await run("Waiting for the payment, then minting") { try await $0.finishTopUp(quoteId: quoteId) }
    }

    /// Swaps a token from an added mint into proofs locked to Prime. No Prime.
    func receive(token: String) async -> UInt64? {
        await run("Receiving the ecash") { try await $0.receive(token: token) }
    }

    /// Prime approves the swap; the answer is a cashuB token.
    func send(mintUrl: String, amountSat: UInt64) async -> CashuSend? {
        await run("Preparing ecash; approve on Prime") { try await $0.send(mintUrl: mintUrl, amountSat: amountSat) }
    }

    /// Swaps an unredeemed sent token back. No Prime.
    func takeBack(sendId: String) async -> UInt64? {
        await run("Taking the ecash back") { try await $0.takeBack(sendId: sendId) }
    }

    /// Prime approves the melt; change comes back from blank outputs.
    func pay(mintUrl: String, invoice: String) async -> CashuPayment? {
        await run("Paying; approve on Prime") { try await $0.pay(mintUrl: mintUrl, invoice: invoice) }
    }
}

/// The link the Rust Cashu wallet calls, from its own threads. It hands the request to the app's
/// one `PrimeBridge`, as `BridgeLink` does for the Fedimint wallet, so a Cashu request shows on the
/// same Prime sheet, and turns the bridge's `LinkError` into `CashuLinkError`.
final class CashuBridgeLink: CashuPrimeLink, @unchecked Sendable {
    private let bridge: PrimeBridge

    init(bridge: PrimeBridge) {
        self.bridge = bridge
    }

    func exchange(request: Data) async throws -> Data {
        do {
            return try await bridge.exchange(request)
        } catch LinkError.Cancelled {
            throw CashuLinkError.Cancelled
        } catch LinkError.Failed(let reason) {
            throw CashuLinkError.Failed(message: reason)
        } catch {
            throw CashuLinkError.Failed(message: error.localizedDescription)
        }
    }
}

extension Error {
    /// The message to show for a failure from the Cashu wallet or its link to Prime; any other
    /// failure as the Fedimint screens show it.
    var cashuShown: String {
        if let error = self as? CashuError {
            switch error {
            case .Invalid(let message), .Prime(let message), .Mint(let message), .Failed(let message):
                return message
            }
        }
        if let error = self as? CashuLinkError {
            switch error {
            case .Cancelled: return "The exchange with Prime was cancelled."
            case .Failed(let message): return message
            }
        }
        return shown
    }
}
#endif
