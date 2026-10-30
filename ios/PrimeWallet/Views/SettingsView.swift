import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("gateway") private var gateway = ""
    @State private var confirmingLeave = false

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
                    Button("Leave federation", role: .destructive) { confirmingLeave = true }
                        .disabled(model.busy != nil)
                }
            }
        }
        .navigationTitle("Settings")
        .scrollContentBackground(.hidden).background(WalletStyle.background)
        .confirmationDialog("Leave this federation?", isPresented: $confirmingLeave, titleVisibility: .visible) {
            Button("Leave", role: .destructive) {
                Task {
                    await model.leave()
                    if model.federation == nil { dismiss() }
                }
            }
        } message: {
            Text("This phone forgets its notes and the pairing. Prime keeps the federation and its keys, so joining it again and pairing recovers the balance.")
        }
    }
}
