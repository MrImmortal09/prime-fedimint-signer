import SwiftUI

/// Every send goes through Prime: it shows what leaves the wallet, and signs once you approve.
struct SendView: View {
    enum Kind: String, CaseIterable, Identifiable {
        case lightning = "Lightning"
        case ecash = "Ecash"
        case onchain = "On chain"
        var id: String { rawValue }
    }

    @EnvironmentObject private var model: WalletModel
    @AppStorage("gateway") private var gateway = ""
    @State private var kind = Kind.lightning
    @State private var invoice = ""
    @State private var amount = ""
    @State private var showOnPrime = false
    @State private var address = ""
    @State private var outcome: String?
    @State private var sent: EcashSend?

    var body: some View {
        Form {
            Picker("Kind", selection: $kind) {
                ForEach(Kind.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .listRowBackground(Color.clear)

            switch kind {
            case .lightning: lightning
            case .ecash: ecash
            case .onchain: onchain
            }
            if let outcome {
                Section { Text(outcome) }
            }
        }
        .navigationTitle("Send")
        .onChange(of: kind) { _ in outcome = nil }
    }

    @ViewBuilder private var lightning: some View {
        Section("Lightning invoice") {
            CodeField(placeholder: "lnbc…", text: $invoice)
        }
        Section {
            Button("Pay with Prime") {
                Task {
                    let value = invoice
                    let chosen = gateway.isEmpty ? nil : gateway
                    if let payment = await model.run("Paying; approve on Prime", {
                        try await $0.payInvoice(invoice: value, gateway: chosen)
                    }) {
                        switch payment {
                        case .paid: outcome = "Paid."
                        case .refunded: outcome = "The gateway could not pay, and the funds came back, less fees."
                        }
                        invoice = ""
                    }
                }
            }
            .disabled(invoice.isEmpty)
        }
    }

    @ViewBuilder private var ecash: some View {
        if let sent {
            Section("Ecash sent") {
                if let token = sent.token {
                    CopyableCode(title: "token", text: token)
                } else {
                    Text("Prime shows the token on its screen. Let the receiver scan it there.")
                }
                Text(Amounts.sats(msat: sent.amountMsat))
                Button("Send more") { self.sent = nil }
            }
        } else {
            Section("Amount") {
                TextField("Sats", text: $amount).keyboardType(.numberPad)
                Toggle("Show the token only on Prime", isOn: $showOnPrime)
            }
            Section {
                Button("Send with Prime") {
                    Task {
                        guard let msat = Amounts.msat(fromSats: amount) else { return }
                        let onPrime = showOnPrime
                        sent = await model.run("Preparing ecash; approve on Prime", {
                            try await $0.sendEcash(amountMsat: msat, showOnPrime: onPrime)
                        })
                    }
                }
                .disabled(Amounts.msat(fromSats: amount) == nil)
            } footer: {
                Text("Prime stages the notes once you approve, then releases them as a token.")
            }
        }
    }

    @ViewBuilder private var onchain: some View {
        Section("Bitcoin address") {
            CodeField(placeholder: "bc1…", text: $address)
            TextField("Sats", text: $amount).keyboardType(.numberPad)
        }
        Section {
            Button("Withdraw with Prime") {
                Task {
                    guard let msat = Amounts.msat(fromSats: amount) else { return }
                    let destination = address
                    if let txid = await model.run("Withdrawing; approve on Prime", {
                        try await $0.withdraw(address: destination, amountSat: msat / 1000)
                    }) {
                        outcome = "Sent on chain: \(txid)"
                        address = ""
                    }
                }
            }
            .disabled(address.isEmpty || Amounts.msat(fromSats: amount) == nil)
        }
    }
}
