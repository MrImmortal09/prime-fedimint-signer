#if canImport(fps_cashuFFI)
import SwiftUI

/// The Cashu wallet: mints, balance, top up, receive, send, pay and activity. Adding a mint,
/// sending and paying need Prime, through the PrimeBridge the Fedimint wallet uses, so they show
/// on the same Prime sheet; topping up, receiving and taking back do not.
struct CashuScreen: View {
    enum Action: String, CaseIterable, Identifiable {
        case topUp, receive, send, pay
        var id: String { rawValue }
        var title: String {
            switch self {
            case .topUp: return "Top up"
            case .receive: return "Receive"
            case .send: return "Send"
            case .pay: return "Pay"
            }
        }
    }

    /// The test mint: cdk-mintd with fake Lightning, so its invoices pay themselves.
    static let testMint = "https://testnut.cashu.space"

    @EnvironmentObject private var cashu: CashuModel
    @EnvironmentObject private var bridge: PrimeBridge
    @AppStorage("primeConnection") private var connection = "qr"
    @AppStorage("simulatorAddress") private var simulatorAddress = ""
    let close: () -> Void

    @State private var action = Action.topUp
    @State private var mintURL = CashuScreen.testMint
    @State private var chosenMint: String?
    @State private var topUpAmount = "100"
    @State private var quote: CashuTopUp?
    @State private var token = ""
    @State private var sendAmount = "10"
    @State private var sent: CashuSend?
    @State private var invoice = ""
    /// What the last call did, until the next one.
    @State private var outcome: String?

    var body: some View {
        NavigationStack {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(WalletStyle.background.ignoresSafeArea())
                .navigationTitle("Cashu")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Close", action: close).accessibilityIdentifier("cashu.close")
                    }
                }
        }
        .disabled(cashu.busy != nil)
        .overlay {
            if let note = cashu.busy, bridge.pending == nil {
                BusyOverlay(note: note)
            }
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { cashu.error != nil }, set: { if !$0 { cashu.error = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(cashu.error ?? "")
        }
        .tint(WalletStyle.copper)
        .foregroundStyle(WalletStyle.ink)
        .preferredColorScheme(.dark)
        .task { await cashu.refresh() }
    }

    @ViewBuilder private var content: some View {
        if let problem = cashu.openError {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.triangle").font(.largeTitle)
                Text("The Cashu wallet could not open").font(.headline)
                Text(problem).font(.footnote).multilineTextAlignment(.center)
                Button("Try again") { Task { await cashu.open(bridge: bridge) } }
                    .buttonStyle(WalletButtonStyle())
                    .accessibilityIdentifier("cashu.retryOpen")
            }
            .padding()
        } else if cashu.wallet == nil {
            ProgressView("Opening the Cashu wallet")
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let outcome {
                        Label(outcome, systemImage: "checkmark.circle")
                            .font(.subheadline).foregroundStyle(WalletStyle.mint)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityIdentifier("cashu.outcome")
                    }
                    balancePanel
                    mintsPanel
                    actionPicker
                    WalletPanel { actionForm }
                    sendsPanel
                    activityPanel
                }
                .padding(20).frame(maxWidth: 650)
                .frame(maxWidth: .infinity)
            }
            .scrollDismissesKeyboard(.interactively)
            .refreshable { await cashu.refresh(reportErrors: true) }
        }
    }

    /// The mint top up, send and pay use: the one tapped in the list, else the first.
    private var currentMint: String? {
        if let chosenMint, cashu.mints.contains(where: { $0.url == chosenMint }) { return chosenMint }
        return cashu.mints.first?.url
    }

    private var balancePanel: some View {
        WalletPanel {
            HStack {
                Label("CASHU", systemImage: "circle.hexagongrid").font(.caption.weight(.semibold))
                Spacer()
                Text(connection == "simulator" && !simulatorAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                     ? "Prime: simulator network" : "Prime: QR codes")
                    .font(.caption).foregroundStyle(WalletStyle.muted)
            }
            .foregroundStyle(WalletStyle.copper)
            Text("Cashu balance").font(.subheadline).foregroundStyle(WalletStyle.muted)
            Text(CashuText.sats(cashu.balanceSat))
                .font(.system(size: 40, weight: .semibold, design: .rounded)).monospacedDigit()
                .minimumScaleFactor(0.5).lineLimit(1)
                .accessibilityLabel("Cashu balance, " + CashuText.sats(cashu.balanceSat))
                .accessibilityValue(String(cashu.balanceSat))
                .accessibilityIdentifier("cashu.balance")
            Text("Your ecash is locked to Passport Prime. Receiving and topping up need no approval; sending and paying do.")
                .font(.caption).foregroundStyle(WalletStyle.muted)
        }
    }

    private var mintsPanel: some View {
        WalletPanel {
            Text("Mints").font(.headline)
            if cashu.mints.isEmpty {
                Text("Add a mint to hold ecash with it. Prime approves each mint once.")
                    .font(.subheadline).foregroundStyle(WalletStyle.muted)
            }
            ForEach(cashu.mints, id: \.url) { mint in
                Button { chosenMint = mint.url } label: {
                    CashuMintRow(mint: mint, chosen: mint.url == currentMint)
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(mint.url == currentMint ? .isSelected : [])
                .accessibilityIdentifier("cashu.mint." + mint.url)
            }
            CashuTextInput(placeholder: "Mint URL", text: $mintURL, identifier: "cashu.addMint.url", keyboard: .URL)
            Button("Add mint") {
                let url = mintURL.trimmingCharacters(in: .whitespacesAndNewlines)
                Task {
                    if let mint = await cashu.addMint(url: url) {
                        chosenMint = mint.url
                        outcome = "Added " + (mint.name ?? mint.url) + "."
                    }
                }
            }
            .buttonStyle(WalletButtonStyle(secondary: true))
            .disabled(mintURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityIdentifier("cashu.addMint")
        }
    }

    private var actionPicker: some View {
        HStack(spacing: 6) {
            ForEach(Action.allCases) { item in
                Button { action = item } label: {
                    Text(item.title)
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .foregroundStyle(action == item ? WalletStyle.background : WalletStyle.ink)
                        .background(action == item ? WalletStyle.copper : WalletStyle.surface,
                                    in: RoundedRectangle(cornerRadius: 12))
                        .contentShape(RoundedRectangle(cornerRadius: 12))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(action == item ? .isSelected : [])
                .accessibilityIdentifier("cashu.tab." + item.rawValue)
            }
        }
    }

    @ViewBuilder private var actionForm: some View {
        switch action {
        case .topUp: topUpForm
        case .receive: receiveForm
        case .send: sendForm
        case .pay: payForm
        }
    }

    private var mintNote: some View {
        Text(currentMint.map { "With " + CashuText.host($0) } ?? "Add a mint first.")
            .font(.subheadline).foregroundStyle(WalletStyle.muted)
            .accessibilityIdentifier("cashu.currentMint")
    }

    @ViewBuilder private var topUpForm: some View {
        Text("Top up by Lightning").font(.headline)
        if let quote {
            Text("Pay this invoice, then finish the top up. The test mint pays its own invoices.")
                .font(.subheadline).foregroundStyle(WalletStyle.muted)
            CashuCodeCard(title: "invoice", text: quote.invoice, identifier: "cashu.topUp.invoice")
            LabeledContent("Amount", value: CashuText.sats(quote.amountSat))
            Button("Finish top up") {
                let id = quote.quoteId
                Task {
                    if let added = await cashu.finishTopUp(quoteId: id) {
                        self.quote = nil
                        outcome = "Topped up " + CashuText.sats(added) + "."
                    }
                }
            }
            .buttonStyle(WalletButtonStyle())
            .accessibilityIdentifier("cashu.topUp.finish")
            Button("Start over") { self.quote = nil }
                .frame(maxWidth: .infinity, minHeight: 44)
                .accessibilityIdentifier("cashu.topUp.cancel")
        } else {
            mintNote
            CashuTextInput(placeholder: "Sats", text: $topUpAmount, identifier: "cashu.topUp.amount", keyboard: .numberPad)
            Button("Get invoice") {
                guard let mint = currentMint, let amount = CashuText.amount(topUpAmount) else { return }
                Task { quote = await cashu.topUp(mintUrl: mint, amountSat: amount) }
            }
            .buttonStyle(WalletButtonStyle())
            .disabled(currentMint == nil || CashuText.amount(topUpAmount) == nil)
            .accessibilityIdentifier("cashu.topUp")
        }
    }

    @ViewBuilder private var receiveForm: some View {
        Text("Receive a token").font(.headline)
        Text("A cashuA or cashuB token from a mint you added. Prime is not needed.")
            .font(.subheadline).foregroundStyle(WalletStyle.muted)
        CashuCodeInput(placeholder: "cashuB…", text: $token, identifier: "cashu.receive.token")
        Button("Receive") {
            let value = token.trimmingCharacters(in: .whitespacesAndNewlines)
            Task {
                if let received = await cashu.receive(token: value) {
                    token = ""
                    outcome = "Received " + CashuText.sats(received) + "."
                }
            }
        }
        .buttonStyle(WalletButtonStyle())
        .disabled(token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        .accessibilityIdentifier("cashu.receive")
    }

    @ViewBuilder private var sendForm: some View {
        Text("Send ecash").font(.headline)
        if let sent {
            Text("This token is money. Anyone who receives it can redeem it.")
                .font(.subheadline).foregroundStyle(WalletStyle.copper)
            CashuCodeCard(title: "token", text: sent.token, identifier: "cashu.send.token")
            LabeledContent("Amount", value: CashuText.sats(sent.amountSat))
            LabeledContent("Fee", value: CashuText.sats(sent.feeSat))
            HStack(spacing: 12) {
                Button("Take back") { takeBack(sent) }
                    .buttonStyle(WalletButtonStyle(secondary: true))
                    .accessibilityIdentifier("cashu.send.takeBack")
                Button("Send more") { self.sent = nil }
                    .buttonStyle(WalletButtonStyle(secondary: true))
                    .accessibilityIdentifier("cashu.send.more")
            }
        } else {
            mintNote
            CashuTextInput(placeholder: "Sats", text: $sendAmount, identifier: "cashu.send.amount", keyboard: .numberPad)
            Button("Send") {
                guard let mint = currentMint, let amount = CashuText.amount(sendAmount) else { return }
                Task { sent = await cashu.send(mintUrl: mint, amountSat: amount) }
            }
            .buttonStyle(WalletButtonStyle())
            .disabled(currentMint == nil || CashuText.amount(sendAmount) == nil)
            .accessibilityIdentifier("cashu.send")
            Text("Prime shows the amount and the mint's fee, and signs once you approve.")
                .font(.footnote).foregroundStyle(WalletStyle.muted)
        }
    }

    @ViewBuilder private var payForm: some View {
        Text("Pay a Lightning invoice").font(.headline)
        mintNote
        CashuCodeInput(placeholder: "lnbc…", text: $invoice, identifier: "cashu.pay.invoice")
        Button("Pay") {
            guard let mint = currentMint else { return }
            let value = invoice.trimmingCharacters(in: .whitespacesAndNewlines)
            Task {
                if let payment = await cashu.pay(mintUrl: mint, invoice: value) {
                    invoice = ""
                    outcome = "Paid \(CashuText.sats(payment.amountSat)), fee \(CashuText.sats(payment.feeSat)), "
                        + "change \(CashuText.sats(payment.changeSat))."
                }
            }
        }
        .buttonStyle(WalletButtonStyle())
        .disabled(currentMint == nil || invoice.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        .accessibilityIdentifier("cashu.pay")
        Text("Prime shows the invoice amount and the fee, and signs once you approve.")
            .font(.footnote).foregroundStyle(WalletStyle.muted)
    }

    @ViewBuilder private var sendsPanel: some View {
        if !cashu.sends.isEmpty {
            WalletPanel {
                Text("Sent tokens").font(.headline)
                Text("Take a token back while nobody has redeemed it.")
                    .font(.subheadline).foregroundStyle(WalletStyle.muted)
                ForEach(cashu.sends, id: \.id) { send in
                    CashuSendRow(send: send) { takeBack(send) }
                }
            }
        }
    }

    private var activityPanel: some View {
        WalletPanel {
            Text("Activity").font(.headline)
            if cashu.activity.isEmpty {
                Text("No Cashu activity yet.").font(.subheadline).foregroundStyle(WalletStyle.muted)
            }
            ForEach(Array(cashu.activity.enumerated()), id: \.offset) { index, item in
                CashuActivityRow(item: item).accessibilityIdentifier("cashu.activity." + String(index))
            }
        }
    }

    private func takeBack(_ send: CashuSend) {
        Task {
            if let back = await cashu.takeBack(sendId: send.id) {
                if sent?.id == send.id { sent = nil }
                outcome = "Took back " + CashuText.sats(back) + "."
            }
        }
    }
}
#endif
