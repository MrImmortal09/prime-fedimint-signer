import SwiftUI

struct RootView: View {
    @EnvironmentObject private var model: WalletModel
    @EnvironmentObject private var bridge: PrimeBridge

    var body: some View {
        NavigationStack {
            content
                .disabled(model.busy != nil)
                .toolbar {
                    ToolbarItem(placement: .primaryAction) {
                        NavigationLink {
                            SettingsView()
                        } label: {
                            Image(systemName: "gearshape").frame(width: 44, height: 44).accessibilityLabel("Settings")
                        }
                    }
                }
        }
        .overlay {
            if let note = model.busy, bridge.pending == nil {
                BusyOverlay(note: note)
            }
        }
        .sheet(item: $bridge.pending) { exchange in
            PrimeExchangeView(exchange: exchange)
                .id(exchange.id)
                .interactiveDismissDisabled()
        }
        .alert(
            "Something went wrong",
            isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.error ?? "")
        }
        .tint(WalletStyle.copper)
        .foregroundStyle(WalletStyle.ink)
        .preferredColorScheme(.dark)
        .task { await model.open() }
    }

    @ViewBuilder private var content: some View {
        if let problem = model.openError {
            VStack(spacing: 12) {
                Image(systemName: "exclamationmark.triangle").font(.largeTitle)
                Text("The wallet could not open").font(.headline)
                Text(problem).font(.footnote).multilineTextAlignment(.center)
                Button("Try again") { Task { await model.open() } }.buttonStyle(WalletButtonStyle())
            }
            .padding()
        } else if model.wallet == nil {
            ProgressView("Opening the wallet")
        } else if model.federation == nil {
            JoinView()
        } else {
            HomeView()
        }
    }
}
