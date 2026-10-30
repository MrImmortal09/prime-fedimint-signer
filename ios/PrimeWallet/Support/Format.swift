import Foundation

enum Amounts {
    private static let formatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        formatter.maximumFractionDigits = 3
        return formatter
    }()

    /// An amount in millisatoshis, as sats.
    static func sats(msat: UInt64) -> String {
        let sats = Double(msat) / 1000
        return (formatter.string(from: NSNumber(value: sats)) ?? "\(sats)") + " sats"
    }

    /// Whole sats typed by the person, as millisatoshis.
    static func msat(fromSats text: String) -> UInt64? {
        guard let sats = UInt64(text.trimmingCharacters(in: .whitespaces)), sats > 0 else { return nil }
        let (msat, overflow) = sats.multipliedReportingOverflow(by: 1000)
        return overflow ? nil : msat
    }
}

extension Error {
    /// The message to show for a failure from the wallet or from Prime.
    var shown: String {
        if let error = self as? WalletError {
            switch error {
            case .NotJoined(let message), .AlreadyJoined(let message), .Invalid(let message),
                 .Prime(let message), .Failed(let message):
                return message
            }
        }
        if let error = self as? LinkError {
            switch error {
            case .Cancelled: return "The exchange with Prime was cancelled."
            case .Failed(let reason): return reason
            }
        }
        return localizedDescription
    }
}
