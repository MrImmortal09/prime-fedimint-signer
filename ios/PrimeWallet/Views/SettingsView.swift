import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: WalletModel
    @AppStorage("gateway") private var gateway = ""

    var body: some View {
        Form {
            Section("Passport Prime") {
                Label(model.federation == nil ? "Pairing needed" : "Wallet paired", systemImage: "qrcode")
                Text("Spending requests travel by QR. Check the details on Prime before approving. Receiving does not require Prime.").font(.subheadline).foregroundStyle(WalletStyle.muted)
            }
            Section("Wallet & appearance") {
                LabeledContent("Protocol", value: "Fedimint")
                LabeledContent("Cashu", value: "Not yet supported")
                LabeledContent("Appearance", value: "Charcoal")
                Text("One federation per wallet is currently supported.").font(.footnote).foregroundStyle(WalletStyle.muted)
            }
            Section("Privacy & trust") {
                Text("Prime holds Fedimint spending keys. Your phone stores wallet activity and view material. The federation is trusted to hold backing funds and redeem ecash.")
                Text("Incoming payments rely on the phone until funds land in Prime-controlled notes. This does not provide trustless Bitcoin self-custody.")
            }
            Section {
                TextField("Federation's gateways", text: $gateway)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
            } header: {
                Text("Lightning gateway")
            } footer: {
                Text("Leave empty to use the gateways the federation lists.")
            }
            if let federation = model.federation {
                Section("Federation") {
                    LabeledContent("Name", value: federation.name ?? "Unnamed")
                    LabeledContent("Guardians", value: "\(federation.guardians)")
                    Text(federation.id).font(.caption.monospaced()).textSelection(.enabled)
                }
            }
        }
        .navigationTitle("Settings")
        .scrollContentBackground(.hidden).background(WalletStyle.background)
    }
}
