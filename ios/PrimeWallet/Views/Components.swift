import SwiftUI

/// A multi-line field for a long code, with Paste and, where the camera can, Scan.
struct CodeField: View {
    let placeholder: String
    @Binding var text: String
    @State private var scanning = false

    var body: some View {
        TextField(placeholder, text: $text, axis: .vertical)
            .lineLimit(2...6)
            .font(.footnote.monospaced())
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
        HStack {
            Button("Paste") {
                text = UIPasteboard.general.string?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            }
            .buttonStyle(.borderless)
            Spacer()
            if Scanner.isAvailable {
                Button("Scan") { scanning = true }
                    .buttonStyle(.borderless)
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
                .frame(maxWidth: 320)
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
            Color.black.opacity(0.25).ignoresSafeArea()
            VStack(spacing: 12) {
                ProgressView()
                Text(note)
                    .multilineTextAlignment(.center)
            }
            .padding(24)
            .background(.regularMaterial)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .padding(32)
        }
    }
}
