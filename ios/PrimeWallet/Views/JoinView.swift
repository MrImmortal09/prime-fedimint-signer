import SwiftUI

struct JoinView: View {
    @EnvironmentObject private var model: WalletModel
    @State private var invite = ""
    @State private var configuring = false
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 28) {
                Label("PRIME ECASH", systemImage: "circle.hexagongrid")
                    .font(.caption.weight(.semibold)).foregroundStyle(WalletStyle.copper)
                Text("Private money.\nReal control.")
                    .font(.system(.largeTitle, design: .rounded).weight(.bold)).fixedSize(horizontal: false, vertical: true)
                Text("Your ecash wallet, protected by Passport Prime.")
                    .font(.title3).foregroundStyle(WalletStyle.muted)
                WalletPanel {
                    FlowStep(number: 1, title: "Connect your wallet", detail: "Your iPhone connects to the federation and handles payments.")
                    FlowStep(number: 2, title: "Pair your Passport", detail: "Prime holds Fedimint spending keys and reviews supported authorizations.")
                    FlowStep(number: 3, title: "Bridge with QR", detail: "Scan between devices. No Bluetooth pairing required.")
                }
                if configuring {
                    WalletPanel {
                        Text("Join a federation").font(.title2.bold())
                        Text("Federation invite code").font(.subheadline.weight(.medium))
                        CodeField(placeholder: "fed1…", text: $invite)
                        Text("Prime will ask you to add the federation, then connect this wallet. Keep both devices nearby.")
                            .font(.subheadline).foregroundStyle(WalletStyle.muted)
                        Button("Join & pair with Prime") { Task { await model.join(invite.trimmingCharacters(in: .whitespacesAndNewlines)) } }
                            .buttonStyle(WalletButtonStyle()).disabled(invite.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.busy != nil)
                    }
                } else {
                    Button("Set up your wallet") { configuring = true }.buttonStyle(WalletButtonStyle())
                }
                Text("Fedimint available · Cashu not yet supported").font(.caption).foregroundStyle(WalletStyle.muted)
                Text("Ecash relies on a mint or federation to redeem your funds. Prime protects spending keys; it does not remove that trust.")
                    .font(.footnote).foregroundStyle(WalletStyle.muted)
            }.padding(24).frame(maxWidth: 600)
        }.background(WalletStyle.background).navigationTitle("").navigationBarTitleDisplayMode(.inline)
    }
}
