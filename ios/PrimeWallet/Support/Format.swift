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

    /// Exact decimal sats for mint-v2's 512-msat ecash denominations.
    /// Parse with integer arithmetic so the amount is never rounded silently.
    static func ecashMsat(fromSats text: String) -> UInt64? {
        var value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if Locale.current.decimalSeparator == "," { value = value.replacingOccurrences(of: ",", with: ".") }
        let parts = value.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...2).contains(parts.count), !parts[0].isEmpty,
              parts.allSatisfy({ $0.utf8.allSatisfy { (48...57).contains($0) } }),
              let whole = UInt64(parts[0]) else { return nil }
        let fraction = parts.count == 2 ? String(parts[1]) : ""
        guard fraction.count <= 3 else { return nil }
        let (base, overflow) = whole.multipliedReportingOverflow(by: 1000)
        guard !overflow, let remainder = UInt64(fraction + String(repeating: "0", count: 3 - fraction.count)) else { return nil }
        let (msat, additionOverflow) = base.addingReportingOverflow(remainder)
        guard !additionOverflow, msat > 0, msat % 512 == 0 else { return nil }
        return msat
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
