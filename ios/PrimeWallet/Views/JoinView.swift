import SwiftUI

struct JoinView: View {
    @EnvironmentObject private var model: WalletModel
    @State private var invite = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                VStack(alignment: .leading, spacing: 10) {
                    Label("ECASH WALLET", systemImage: "circle.hexagongrid")
                        .font(.caption.weight(.semibold)).foregroundStyle(WalletStyle.copper)
                    Text("Connect a federation")
                        .font(.title.bold())
                    Text("Add your invite code. Approve the federation, then the wallet connection on Prime.")
                        .font(.subheadline).foregroundStyle(WalletStyle.muted)
                }
                VStack(alignment: .leading, spacing: 12) {
                    Text("Federation invite").font(.subheadline.weight(.semibold))
                    CodeField(placeholder: "fed1…", text: $invite)
                    Button("Connect to Prime") {
                        Task { await model.join(invite.trimmingCharacters(in: .whitespacesAndNewlines)) }
                    }
                    .buttonStyle(WalletButtonStyle())
                    .disabled(invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.busy != nil)
                }
                Divider().overlay(WalletStyle.muted.opacity(0.2))
                VStack(alignment: .leading, spacing: 14) {
                    Label("Your phone receives payments", systemImage: "arrow.down.left")
                    Label("Prime approves spending", systemImage: "checkmark.shield")
                    Label("Connect by QR or simulator Wi-Fi", systemImage: "qrcode")
                }.font(.subheadline).foregroundStyle(WalletStyle.muted)
                Text("Your federation holds the backing funds. Prime protects your spending keys.")
                    .font(.footnote).foregroundStyle(WalletStyle.muted)
#if canImport(fps_cashuFFI)
                Text("Fedimint · Cashu available")
                    .font(.caption).foregroundStyle(WalletStyle.muted)
#else
                Text("Fedimint · Cashu not yet supported")
                    .font(.caption).foregroundStyle(WalletStyle.muted)
#endif
            }
            .padding(20).frame(maxWidth: 600)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(WalletStyle.background)
        .navigationTitle("").navigationBarTitleDisplayMode(.inline)
    }
}
