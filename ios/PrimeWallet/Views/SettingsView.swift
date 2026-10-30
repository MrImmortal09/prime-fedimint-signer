import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("gateway") private var gateway = ""
    @State private var confirmingLeave = false

    var body: some View {
        Form {
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
