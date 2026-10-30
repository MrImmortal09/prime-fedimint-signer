import SwiftUI

/// Receiving needs no Prime: the phone derives every key a payment lands on from Prime's view material.
struct ReceiveView: View {
    enum Kind: String, CaseIterable, Identifiable {
        case ecash = "Ecash"
        case lightning = "Lightning"
        case onchain = "On chain"
        var id: String { rawValue }
    }

    @EnvironmentObject private var model: WalletModel
    @AppStorage("gateway") private var gateway = ""
    @State private var kind = Kind.ecash
    @State private var token = ""
    @State private var amount = ""
    @State private var note = ""
    @State private var invoice: LightningInvoice?
    @State private var invoicePaid: Bool?
    @State private var address: String?
    @State private var received: String?

    var body: some View {
        Form {
            Picker("Kind", selection: $kind) {
                ForEach(Kind.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .listRowBackground(Color.clear)

            switch kind {
            case .ecash: ecash
            case .lightning: lightning
            case .onchain: onchain
            }
        }
        .navigationTitle("Receive")
    }

    @ViewBuilder private var ecash: some View {
        Section("Ecash token") {
            CodeField(placeholder: "fedimint1…", text: $token)
        }
        Section {
            Button("Redeem") {
                Task {
                    let value = token
                    if let msat = await model.run("Redeeming the ecash", { try await $0.receiveEcash(token: value) }) {
                        received = "Received \(Amounts.sats(msat: msat))"
                        token = ""
                    }
                }
            }
            .disabled(token.isEmpty)
            if let received { Text(received).foregroundStyle(.green) }
        }
    }

    @ViewBuilder private var lightning: some View {
        if let invoice {
            Section("Invoice") {
                CopyableCode(title: "invoice", text: invoice.invoice)
                switch invoicePaid {
                case .none: Label("Waiting for the payment", systemImage: "hourglass")
                case .some(true): Label("Paid", systemImage: "checkmark.circle").foregroundStyle(.green)
                case .some(false): Label("Expired", systemImage: "xmark.circle").foregroundStyle(.red)
                }
            }
        } else {
            Section("Amount") {
                TextField("Sats", text: $amount).keyboardType(.numberPad)
                TextField("Description", text: $note)
            }
            Section {
                Button("Create invoice") { Task { await createInvoice() } }
                    .disabled(Amounts.msat(fromSats: amount) == nil)
            }
        }
    }

    @ViewBuilder private var onchain: some View {
        if let address {
            Section("Deposit address") {
                CopyableCode(title: "address", text: address)
            }
            Section {
                Text("The deposit counts once the federation sees it final, after several blocks.")
                    .font(.footnote)
            }
        } else {
            Section {
                Button("Show a deposit address") {
                    Task { address = await model.run("Making a deposit address") { try await $0.depositAddress() } }
                }
            }
        }
    }

    private func createInvoice() async {
        guard let msat = Amounts.msat(fromSats: amount) else { return }
        let description = note
        let chosen = gateway.isEmpty ? nil : gateway
        guard let created = await model.run("Asking the gateway for an invoice", {
            try await $0.receiveLightning(amountMsat: msat, description: description, gateway: chosen)
        }) else { return }
        invoice = created
        invoicePaid = nil
        guard let wallet = model.wallet else { return }
        do {
            invoicePaid = try await wallet.awaitLightningReceive(operationId: created.operationId)
            await model.refresh()
        } catch {
            model.error = error.shown
        }
    }
}
