import SwiftUI

/// The first screen: join a federation and pair it with Prime.
struct JoinView: View {
    @EnvironmentObject private var model: WalletModel
    @State private var invite = ""

    var body: some View {
        Form {
            Section {
                Text("This wallet holds no spend keys. Passport Prime holds them, and signs a spend only after you approve it on its screen.")
            }
            Section("Federation invite code") {
                CodeField(placeholder: "fed1…", text: $invite)
            }
            Section {
                Button("Join and pair with Prime") {
                    Task { await model.join(invite.trimmingCharacters(in: .whitespacesAndNewlines)) }
                }
                .disabled(invite.isEmpty || model.busy != nil)
            } footer: {
                Text("Prime asks you to approve the federation, then to connect this wallet.")
            }
        }
        .navigationTitle("Prime Wallet")
    }
}
