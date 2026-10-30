import SwiftUI

struct HomeView: View {
    @EnvironmentObject private var model: WalletModel

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 4) {
                    Text(Amounts.sats(msat: model.balanceMsat))
                        .font(.largeTitle.bold())
                    Text(federationName)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 8)
            }
            Section {
                NavigationLink {
                    ReceiveView()
                } label: {
                    Label("Receive", systemImage: "arrow.down.circle")
                }
                NavigationLink {
                    SendView()
                } label: {
                    Label("Send", systemImage: "arrow.up.circle")
                }
            }
            if !model.pendingSends.isEmpty {
                Section {
                    ForEach(model.pendingSends, id: \.id) { send in
                        PendingSendRow(send: send)
                    }
                } header: {
                    Text("Ecash sent")
                } footer: {
                    Text("Take back ecash the receiver never redeemed.")
                }
            }
            Section("Activity") {
                if model.activity.isEmpty {
                    Text("Nothing yet").foregroundStyle(.secondary)
                }
                ForEach(Array(model.activity.enumerated()), id: \.offset) { _, item in
                    HStack {
                        Text(item.title)
                        Spacer()
                        Text(Date(timeIntervalSince1970: TimeInterval(item.timeUnix)), style: .relative)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle("Prime Wallet")
        .refreshable { await model.refresh() }
    }

    private var federationName: String {
        guard let federation = model.federation else { return "" }
        return federation.name ?? "Federation \(federation.id.prefix(8))"
    }
}

struct PendingSendRow: View {
    @EnvironmentObject private var model: WalletModel
    let send: PendingSend
    @State private var showingToken = false

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
                            model.error = nil
                            _ = amount
                        }
                    }
                }
                Spacer()
                if send.token != nil {
                    Button("Show token") { showingToken = true }
                }
                Spacer()
                Button("Forget", role: .destructive) {
                    Task { await model.run("Forgetting the send") { try await $0.forgetSend(id: send.id) } }
                }
            }
            .buttonStyle(.borderless)
        }
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
