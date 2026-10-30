import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("gateway") private var gateway = ""
    @AppStorage("primeConnection") private var connection = "qr"
    @AppStorage("simulatorAddress") private var simulatorAddress = ""
    @State private var confirmingLeave = false

    var body: some View {
        Form {
            Section("Passport Prime") {
                Label(model.federation == nil ? "Pairing needed" : "Wallet paired", systemImage: "qrcode")
                Text("Check the details on Prime before approving spending requests. Receiving does not require Prime.").font(.subheadline).foregroundStyle(WalletStyle.muted)
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
                Picker("Connection", selection: $connection) {
                    Text("QR codes").tag("qr")
                    Text("Simulator over Wi-Fi").tag("simulator")
                }
                if connection == "simulator" {
                    TextField("Mac's address, e.g. 192.168.1.5", text: $simulatorAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                }
            } header: {
                Text("Passport Prime")
            } footer: {
                Text(connection == "simulator"
                    ? "Requests go to the app in the KeyOS simulator on your Mac, on the same Wi-Fi, port 7865 unless given. A stand-in for Bluetooth while there is no device."
                    : "Requests travel as animated QR codes that Prime scans, and Prime's answers are read back with the camera.")
            }
            .onChange(of: connection) { _ in model.applyConnection() }
            .onChange(of: simulatorAddress) { _ in model.applyConnection() }
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
