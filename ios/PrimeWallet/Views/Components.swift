import SwiftUI

/// A multi-line field for a long code, with Paste and, where the camera can, Scan.
struct CodeField: View {
    let placeholder: String
    @Binding var text: String
    @State private var scanning = false
    @State private var pasteNotice: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField(placeholder, text: $text, axis: .vertical)
                .lineLimit(2...3)
                .font(.subheadline.monospaced())
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .accessibilityLabel(placeholder == "fed1…" ? "Federation invite code" : "Payment code")
                .padding(14)
                .frame(maxWidth: .infinity, minHeight: 76, alignment: .topLeading)
                .background(WalletStyle.surface, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
            HStack(spacing: 10) {
                Button {
                    if let value = UIPasteboard.general.string, !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        text = value.trimmingCharacters(in: .whitespacesAndNewlines)
                        pasteNotice = nil
                    } else {
                        pasteNotice = "No text to paste. Copy a code first."
                    }
                } label: {
                    Label("Paste", systemImage: "doc.on.clipboard")
                        .frame(maxWidth: .infinity, minHeight: 48)
                }
                .buttonStyle(.plain)
                .background(WalletStyle.surface, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
                if Scanner.isAvailable {
                    Button { scanning = true } label: {
                        Label("Scan", systemImage: "qrcode.viewfinder")
                            .frame(maxWidth: .infinity, minHeight: 48)
                    }
                    .buttonStyle(.plain)
                    .background(WalletStyle.surface, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(WalletStyle.muted.opacity(0.25)))
                    .accessibilityLabel("Scan a QR code")
                }
            }
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

/// A value to show in full and copy: an invoice, a token, an address.
struct CopyableCode: View {
    let title: String
    let text: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            QRCodeView(text: text)
                .padding(16).background(.white, in: RoundedRectangle(cornerRadius: 20))
                .frame(maxWidth: 320)
                .accessibilityLabel("QR code for " + title)
                .frame(maxWidth: .infinity)
            Text(text)
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .lineLimit(4)
            HStack {
                Button(copied ? "Copied" : "Copy \(title)") {
                    UIPasteboard.general.string = text
                    copied = true
                }
                Spacer()
                ShareLink(item: text)
            }
            .buttonStyle(.borderless)
        }
    }
}

/// Covers the screen while the wallet works on something.
struct BusyOverlay: View {
    let note: String

    var body: some View {
        ZStack {
            Color.black.opacity(0.7).ignoresSafeArea()
            VStack(spacing: 12) {
                ProgressView()
                Text(note)
                    .multilineTextAlignment(.center)
            }
            .padding(24)
            .background(WalletStyle.surface)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .padding(32)
        }
    }
}
