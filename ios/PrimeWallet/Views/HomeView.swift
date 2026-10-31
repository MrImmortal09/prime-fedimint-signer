import SwiftUI

struct HomeView: View {
    @EnvironmentObject private var model: WalletModel
    @AppStorage("primeConnection") private var connection = "qr"
    @AppStorage("simulatorAddress") private var simulatorAddress = ""
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Label("FEDIMINT", systemImage: "circle.hexagongrid").font(.caption.weight(.semibold))
                    Spacer()
#if canImport(fps_cashuFFI)
                    Text("Cashu · Available").font(.caption).foregroundStyle(WalletStyle.muted)
#else
                    Text("Cashu · Unavailable").font(.caption).foregroundStyle(WalletStyle.muted)
#endif
                }.foregroundStyle(WalletStyle.copper)
                WalletPanel {
                    Text("Available balance").font(.subheadline).foregroundStyle(WalletStyle.muted)
                    Text(Amounts.sats(msat: model.balanceMsat))
                        .font(.system(size: 44, weight: .semibold, design: .rounded)).monospacedDigit()
                        .minimumScaleFactor(0.5).lineLimit(1)
                        .accessibilityLabel("Available balance, " + Amounts.sats(msat: model.balanceMsat))
                    Divider()
                    Label(federationName, systemImage: "building.2").font(.subheadline)
                    Label("Paired with Passport Prime", systemImage: "checkmark.shield")
                        .font(.caption).foregroundStyle(WalletStyle.mint)
                    Text(connection == "simulator" && !simulatorAddress.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                         ? "Authorization uses the simulator network connection."
                         : "Authorization uses QR. Prime does not need to stay connected.").font(.caption).foregroundStyle(WalletStyle.muted)
                }
                HStack(spacing: 12) {
                    NavigationLink { SendView() } label: { Label("Send", systemImage: "arrow.up.right") }
                        .buttonStyle(WalletButtonStyle())
                    NavigationLink { ReceiveView() } label: { Label("Receive", systemImage: "arrow.down.left") }
                        .buttonStyle(WalletButtonStyle(secondary: true))
                }
                if !model.pendingSends.isEmpty {
                    Text("Ecash sends").font(.title2.bold())
                    Text("Stored sends are separate from your available balance. Redemption status is not tracked here.")
                        .font(.subheadline).foregroundStyle(WalletStyle.muted)
                    ForEach(model.pendingSends, id: \.id) { send in WalletPanel { PendingSendRow(send: send) } }
                }
                HStack {
                    Text("Recent activity").font(.title2.bold())
                    Spacer()
                    NavigationLink("View all") { ActivityView() }.frame(minHeight: 44)
                }
                if model.activity.isEmpty {
                    WalletPanel { WalletNotice(title: "Your first payment starts here", message: "Receive ecash, request a Lightning payment, or make a Bitcoin deposit.", symbol: "arrow.down.left") }
                } else {
                    WalletPanel {
                        ForEach(Array(model.activity.prefix(3).enumerated()), id: \.offset) { _, item in
                            NavigationLink { ActivityDetail(item: item) } label: { ActivityRow(item: item) }.buttonStyle(.plain)
                        }
                    }
                }
                Text("Ecash is backed by your federation. Learn about trust and privacy in Settings.")
                    .font(.footnote).foregroundStyle(WalletStyle.muted)
            }.padding(20).frame(maxWidth: 650)
        }.background(WalletStyle.background).navigationTitle("Ecash Wallet").navigationBarTitleDisplayMode(.inline)
            .refreshable { await model.refresh() }
    }
    private var federationName: String {
        model.federation?.name ?? "Federation " + (model.federation.map { String($0.id.prefix(8)) } ?? "")
    }
}

struct ActivityRow: View {
    let item: Activity
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: item.module == "lnv2" ? "bolt" : item.module == "walletv2" ? "bitcoinsign.circle" : "circle.hexagongrid")
                .font(.title3).foregroundStyle(WalletStyle.copper).frame(width: 40, height: 44).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title).font(.subheadline.weight(.medium))
                Text(Date(timeIntervalSince1970: TimeInterval(item.timeUnix)), style: .relative)
                    .font(.caption).foregroundStyle(WalletStyle.muted)
            }
            Spacer()
            Image(systemName: "chevron.right").font(.caption).foregroundStyle(WalletStyle.muted).accessibilityHidden(true)
        }.frame(minHeight: 48)
    }
}

struct ActivityView: View {
    @EnvironmentObject private var model: WalletModel
    var body: some View {
        List {
            if model.activity.isEmpty { Text("No activity yet").foregroundStyle(WalletStyle.muted) }
            ForEach(Array(model.activity.enumerated()), id: \.offset) { _, item in
                NavigationLink { ActivityDetail(item: item) } label: { ActivityRow(item: item) }
            }
        }.scrollContentBackground(.hidden).background(WalletStyle.background).navigationTitle("Activity")
    }
}

struct ActivityDetail: View {
    let item: Activity
    var body: some View {
        Form {
            Section("Operation") {
                Text(item.title).font(.headline)
                LabeledContent("Method", value: item.module == "lnv2" ? "Lightning" : item.module == "walletv2" ? "On-chain Bitcoin" : "Ecash")
                Text(Date(timeIntervalSince1970: TimeInterval(item.timeUnix)), style: .date)
                Text(Date(timeIntervalSince1970: TimeInterval(item.timeUnix)), style: .time)
            }
            Section { Text("The wallet history currently provides the operation title, method, and time. Amount, fees, and final status are not available in this record.").font(.subheadline).foregroundStyle(WalletStyle.muted) }
        }.navigationTitle("Activity details").scrollContentBackground(.hidden).background(WalletStyle.background)
    }
}

struct PendingSendRow: View {
    @EnvironmentObject private var model: WalletModel
    let send: PendingSend
    @State private var showingToken = false
    @State private var forgetting = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(Amounts.sats(msat: send.amountMsat))
                Spacer()
                Text(Date(timeIntervalSince1970: TimeInterval(send.createdUnix)), style: .relative)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Text(status)
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Button("Take back") {
                    Task {
                        if let amount = await model.run("Taking the ecash back", { try await $0.reclaim(id: send.id) }) {
                            model.notice = Amounts.sats(msat: amount) + " is back in your balance, less the federation's fee."
                        }
                    }
                }
                Spacer()
                if send.token != nil {
                    Button("Show token") { showingToken = true }
                }
                Spacer()
                Button("Forget", role: .destructive) {
                    forgetting = true
                }
            }
            .buttonStyle(.borderless)
        }
        .confirmationDialog("Forget this send?", isPresented: $forgetting, titleVisibility: .visible) {
            Button("Forget tracking record", role: .destructive) {
                Task { await model.run("Forgetting the send") { try await $0.forgetSend(id: send.id) } }
            }
        } message: { Text("This removes the record from your phone. It does not reclaim ecash or return funds.") }
        .sheet(isPresented: $showingToken) {
            NavigationStack {
                ScrollView {
                    CopyableCode(title: "token", text: send.token ?? "")
                        .padding()
                }
                .navigationTitle("Ecash token")
                .navigationBarTitleDisplayMode(.inline)
            }
        }
    }

    private var status: String {
        if !send.released { return "Staged on Prime, not released" }
        return send.token == nil ? "Shown on Prime's screen" : "Token on this phone"
    }
}
