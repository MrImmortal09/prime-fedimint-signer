import Foundation

/// A request waiting for Prime while the exchange screen is up.
struct PendingExchange: Identifiable {
    let id = UUID()
    /// The animated QR codes that carry the request.
    let frames: [String]
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
        let frames = requestFrames(request: request)
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            self.pending = PendingExchange(frames: frames)
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
