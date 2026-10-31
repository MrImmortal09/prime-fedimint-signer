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
    @State private var reviewing = false
    @State private var sendAfterReview = false

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
                Section("Payment update") {
                    Label(outcome, systemImage: "info.circle").font(.body)
                }
            }
        }
        .navigationTitle("Send")
        .scrollContentBackground(.hidden).background(WalletStyle.background)
        .disabled(model.busy != nil)
        .sheet(isPresented: $reviewing, onDismiss: {
            if sendAfterReview { sendAfterReview = false; performSend() }
        }) {
            NavigationStack {
                Form {
                    Section("Payment method") { Text(kind.rawValue).font(.headline) }
                    Section("You are requesting") {
                        if kind == .lightning { Text(invoice).font(.footnote.monospaced()).textSelection(.enabled) }
                        else { LabeledContent("Amount", value: amount + " sats") }
                        if kind == .onchain { Text(address).font(.footnote.monospaced()).textSelection(.enabled) }
                        if kind == .ecash { LabeledContent("Token delivery", value: showOnPrime ? "Prime screen only" : "This phone") }
                    }
                    Section {
                        Text("Final transaction details and fees are reviewed on Passport Prime. Verify them there before sliding to approve.")
                        Button("Continue to Passport") { sendAfterReview = true; reviewing = false }
                            .buttonStyle(WalletButtonStyle())
                    }
                }.scrollContentBackground(.hidden).background(WalletStyle.background)
                    .navigationTitle("Review payment").navigationBarTitleDisplayMode(.inline)
                    .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Back") { reviewing = false } } }
            }.preferredColorScheme(.dark).tint(WalletStyle.copper)
        }
        .onChange(of: kind) { _ in outcome = nil }
    }

    @ViewBuilder private var lightning: some View {
        Section("Lightning invoice") {
            CodeField(placeholder: "lnbc…", text: $invoice)
        }
        Section {
            Button("Review Lightning payment") { reviewing = true }
            .disabled(invoice.isEmpty)
        }
    }

    @ViewBuilder private var ecash: some View {
        if let sent {
            Section("Ecash ready to share") {
                if let token = sent.token {
                    Text("This token is money. Anyone who receives it can redeem it.").font(.subheadline).foregroundStyle(WalletStyle.copper)
                    CopyableCode(title: "token", text: token)
                } else {
                    Text("Prime shows the token on its screen. Let the receiver scan it there.")
                }
                Text(Amounts.sats(msat: sent.amountMsat))
                Button("Send more") { self.sent = nil }
            }
        } else {
            Section("Amount") {
                TextField("Sats", text: $amount).keyboardType(.decimalPad)
                Text("Ecash uses 0.512-sat increments. For whole sats, use a multiple of 64, such as 128 sats.")
                    .font(.footnote).foregroundStyle(WalletStyle.muted)
                if !amount.isEmpty && Amounts.ecashMsat(fromSats: amount) == nil {
                    Text("Enter an exact ecash amount, such as 100.352 or 128 sats.")
                        .font(.footnote).foregroundStyle(WalletStyle.danger)
                }
                Toggle("Show the token only on Prime", isOn: $showOnPrime)
            }
            Section {
                Button("Review ecash send") { reviewing = true }
                .disabled(Amounts.ecashMsat(fromSats: amount) == nil)
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
            Button("Review withdrawal") { reviewing = true }
            .disabled(address.isEmpty || Amounts.msat(fromSats: amount) == nil)
        }
    }
    private func performSend() {
        outcome = nil
        Task {
            switch kind {
            case .lightning:
                let chosen = gateway.isEmpty ? nil : gateway
                let value = invoice
                if let payment = await model.run("Paying; approve on Prime", {
                    try await $0.payInvoice(invoice: value, gateway: chosen)
                }) {
                    switch payment {
                    case .paid: outcome = "Payment complete. The gateway confirmed payment."
                    case .refunded: outcome = "The gateway could not pay. Funds returned, less fees."
                    }
                    invoice = ""
                }
            case .ecash:
                guard let msat = Amounts.ecashMsat(fromSats: amount) else { return }
                let onPrime = showOnPrime
                sent = await model.run("Preparing ecash; approve on Prime", {
                    try await $0.sendEcash(amountMsat: msat, showOnPrime: onPrime)
                })
            case .onchain:
                guard let msat = Amounts.msat(fromSats: amount) else { return }
                let destination = address
                if let txid = await model.run("Withdrawing; approve on Prime", {
                    try await $0.withdraw(address: destination, amountSat: msat / 1000)
                }) {
                    outcome = "Withdrawal submitted. Bitcoin confirmations may still be pending. Transaction: \(txid)"
                    address = ""
                }
            }
        }
    }
}
