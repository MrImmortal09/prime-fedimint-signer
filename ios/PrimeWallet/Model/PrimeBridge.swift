import Foundation

/// A request waiting for Prime while the exchange screen is up.
struct PendingExchange: Identifiable {
    let id = UUID()
    /// The request bytes; the exchange screen turns them into animated QR codes at the size the
    /// person chose.
    let request: Data

    /// Read only the canonical request tag; the Rust core owns all protocol validation.
    private var requestTag: String? {
        let header = Data([0xa3, 0x61, 0x76, 0x01, 0x62, 0x69, 0x64, 0x50])
        let body = Data([0x64, 0x62, 0x6f, 0x64, 0x79, 0xa1])
        guard request.starts(with: header), request.count > 30,
              request.dropFirst(24).starts(with: body) else { return nil }
        let length = Int(request[30]) - 0x60
        guard (0..<24).contains(length), request.count >= 31 + length else { return nil }
        return String(data: request.subdata(in: 31..<(31 + length)), encoding: .utf8)
    }

    var pairingStep: Int? {
        switch requestTag {
        case "ImportFederation": return 1
        case "ExportViewMaterial": return 2
        default: return nil
        }
    }

    var title: String {
        switch pairingStep {
        case 1: return "Add your federation"
        case 2: return "Connect your wallet"
        default: return "Review on Passport Prime"
        }
    }

    var instructions: String {
        switch pairingStep {
        case 1: return "Pairing needs two approvals. First, scan this code on Prime and approve adding your federation."
        case 2: return "Federation added. On Prime, tap Scan next request, scan this new code, then approve Connect wallet app."
        default: return "On Prime, scan this code. Check the details, then slide to approve."
        }
    }
}

/// Carries the wallet's requests to Passport Prime as animated QR codes, and its answers back.
/// The Rust wallet calls it through `BridgeLink` whenever a spend needs Prime, including from
/// background work such as a refund, so the exchange screen can appear at any time.
@MainActor
final class PrimeBridge: ObservableObject {
    @Published var pending: PendingExchange?
    private var continuation: CheckedContinuation<Data, Error>?

    func exchange(_ request: Data) async throws -> Data {
        if continuation != nil {
            throw LinkError.Failed(reason: "another exchange with Prime is in progress")
        }
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            self.pending = PendingExchange(request: request)
        }
    }

    /// Prime's answer, read off its screen.
    func answer(_ answer: Data) {
        continuation?.resume(returning: answer)
        continuation = nil
        pending = nil
    }

    func cancel() {
        continuation?.resume(throwing: LinkError.Cancelled)
        continuation = nil
        pending = nil
    }
}

/// The link the Rust wallet calls, from its own threads.
final class BridgeLink: PrimeLink, @unchecked Sendable {
    private let bridge: PrimeBridge

    init(bridge: PrimeBridge) {
        self.bridge = bridge
    }

    func exchange(request: Data) async throws -> Data {
        try await bridge.exchange(request)
    }
}
