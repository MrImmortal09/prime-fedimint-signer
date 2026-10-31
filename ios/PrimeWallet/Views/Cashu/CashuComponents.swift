#if canImport(fps_cashuFFI)
import SwiftUI

/// Text for Cashu amounts, mints and activity.
enum CashuText {
    private static let formatter: NumberFormatter = {
        let formatter = NumberFormatter()
        formatter.numberStyle = .decimal
        return formatter
    }()

    static func sats(_ value: UInt64) -> String {
        (formatter.string(from: NSNumber(value: value)) ?? "\(value)") + (value == 1 ? " sat" : " sats")
    }

    /// Whole sats typed by the person; nil unless more than zero.
    static func amount(_ text: String) -> UInt64? {
        guard let value = UInt64(text.trimmingCharacters(in: .whitespacesAndNewlines)), value > 0 else { return nil }
        return value
    }

    /// A mint's host, to name it briefly.
    static func host(_ url: String) -> String {
        URL(string: url)?.host ?? url
    }

    /// An activity kind such as `top_up`, as a title.
    static func title(kind: String) -> String {
        let words = kind.replacingOccurrences(of: "_", with: " ")
        return words.prefix(1).uppercased() + words.dropFirst()
    }

    static func symbol(kind: String) -> String {
        let kind = kind.lowercased()
        if kind.contains("pay") || kind.contains("melt") { return "bolt" }
        if kind.contains("back") || kind.contains("reclaim") { return "arrow.uturn.backward" }
        if kind.contains("send") { return "arrow.up.right" }
        if kind.contains("receive") || kind.contains("top") { return "arrow.down.left" }
        return "circle.hexagongrid"
    }
}

/// A one-line field: a mint URL or an amount.
struct CashuTextInput: View {
    let placeholder: String
    @Binding var text: String
    let identifier: String
    var keyboard: UIKeyboardType = .default

    var body: some View {
        TextField(placeholder, text: $text)
            .keyboardType(keyboard)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(.body.monospaced())
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity, minHeight: 50)
            .background(WalletStyle.background, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
            .accessibilityLabel(placeholder)
            .accessibilityIdentifier(identifier)
    }
}

/// A field for a token or an invoice, with Paste and, where the camera can, Scan.
struct CashuCodeInput: View {
    let placeholder: String
    @Binding var text: String
    let identifier: String
    @State private var scanning = false
    @State private var pasteNotice: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField(placeholder, text: $text, axis: .vertical)
                .lineLimit(2...4)
                .font(.subheadline.monospaced())
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(14)
                .frame(maxWidth: .infinity, minHeight: 76, alignment: .topLeading)
                .background(WalletStyle.background, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
                .accessibilityIdentifier(identifier)
            HStack(spacing: 10) {
                Button {
                    let value = UIPasteboard.general.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    if value.isEmpty {
                        pasteNotice = "No text to paste. Copy a code first."
                    } else {
                        text = value
                        pasteNotice = nil
                    }
                } label: {
                    Label("Paste", systemImage: "doc.on.clipboard").modifier(CashuSmallButton())
                }
                .accessibilityIdentifier(identifier + ".paste")
                if Scanner.isAvailable {
                    Button { scanning = true } label: {
                        Label("Scan", systemImage: "qrcode.viewfinder").modifier(CashuSmallButton())
                    }
                    .accessibilityIdentifier(identifier + ".scan")
                }
            }
            .buttonStyle(.plain)
            .font(.subheadline.weight(.semibold))
            if let pasteNotice {
                Text(pasteNotice).font(.caption).foregroundStyle(WalletStyle.muted)
            }
        }
        .sheet(isPresented: $scanning) {
            ScanSheet { code in text = code }
        }
    }
}

/// The look of Paste and Scan, as `CodeField` draws them.
struct CashuSmallButton: ViewModifier {
    func body(content: Content) -> some View {
        content
            .frame(maxWidth: .infinity, minHeight: 44)
            .background(WalletStyle.background, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
            .contentShape(RoundedRectangle(cornerRadius: 12))
    }
}

/// An invoice or a token to hand on: QR code, the text, Copy and Share.
struct CashuCodeCard: View {
    let title: String
    let text: String
    let identifier: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            QRCodeView(text: text)
                .frame(maxWidth: 260)
                .frame(maxWidth: .infinity)
                .accessibilityLabel("QR code for the " + title)
            Text(text)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .lineLimit(4)
                .accessibilityIdentifier(identifier)
            HStack {
                Button(copied ? "Copied" : "Copy " + title) {
                    UIPasteboard.general.string = text
                    copied = true
                }
                .accessibilityIdentifier(identifier + ".copy")
                Spacer()
                ShareLink(item: text) { Label("Share", systemImage: "square.and.arrow.up") }
                    .accessibilityIdentifier(identifier + ".share")
            }
            .buttonStyle(.borderless)
            .frame(minHeight: 44)
        }
        .onChange(of: text) { _ in copied = false }
    }
}

struct CashuMintRow: View {
    let mint: CashuMint
    let chosen: Bool

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: chosen ? "largecircle.fill.circle" : "circle")
                .foregroundStyle(WalletStyle.copper).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(mint.name ?? CashuText.host(mint.url)).font(.subheadline.weight(.medium))
                Text(mint.url).font(.caption).foregroundStyle(WalletStyle.muted).lineLimit(1)
            }
            Spacer()
            Text(CashuText.sats(mint.balanceSat)).font(.subheadline).monospacedDigit()
        }
        .frame(minHeight: 48)
        .contentShape(Rectangle())
    }
}

/// A sent token Prime approved, until it is taken back.
struct CashuSendRow: View {
    let send: CashuSend
    let takeBack: () -> Void
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(CashuText.sats(send.amountSat)).font(.subheadline.weight(.medium)).monospacedDigit()
                Spacer()
                Text(CashuText.host(send.mintUrl) + " · fee " + CashuText.sats(send.feeSat))
                    .font(.caption).foregroundStyle(WalletStyle.muted)
            }
            HStack {
                Button(copied ? "Copied" : "Copy token") {
                    UIPasteboard.general.string = send.token
                    copied = true
                }
                .accessibilityIdentifier("cashu.sends.copy." + send.id)
                Spacer()
                ShareLink(item: send.token) { Label("Share", systemImage: "square.and.arrow.up") }
                    .accessibilityIdentifier("cashu.sends.share." + send.id)
                Spacer()
                Button("Take back", action: takeBack)
                    .accessibilityIdentifier("cashu.sends.takeBack." + send.id)
            }
            .buttonStyle(.borderless)
            .font(.subheadline)
            .frame(minHeight: 44)
        }
    }
}

struct CashuActivityRow: View {
    let item: CashuActivity

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: CashuText.symbol(kind: item.kind))
                .font(.title3).foregroundStyle(WalletStyle.copper).frame(width: 36).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(CashuText.title(kind: item.kind)).font(.subheadline.weight(.medium))
                if !item.detail.isEmpty {
                    Text(item.detail).font(.caption).foregroundStyle(WalletStyle.muted).lineLimit(2)
                }
                HStack(spacing: 4) {
                    Text(CashuText.host(item.mintUrl))
                    Text("·")
                    Text(Date(timeIntervalSince1970: TimeInterval(item.unixTime)), style: .relative)
                }
                .font(.caption).foregroundStyle(WalletStyle.muted)
            }
            Spacer()
            Text(CashuText.sats(item.amountSat)).font(.subheadline).monospacedDigit()
        }
        .frame(minHeight: 48)
        .accessibilityElement(children: .combine)
    }
}
#endif
