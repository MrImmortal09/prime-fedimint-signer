#if canImport(fps_cashuFFI)
import SwiftUI

/// The layer above the unchanged `RootView`: a small floating Cashu button, and the Cashu screen it
/// opens over everything else. RootView keeps the Prime exchange sheet, and the Cashu wallet uses
/// the same `PrimeBridge`, so every request to Prime, Fedimint or Cashu, shows on that one sheet,
/// presented above this layer.
struct CashuLayer: View {
    @EnvironmentObject private var cashu: CashuModel
    @EnvironmentObject private var wallet: WalletModel
    @EnvironmentObject private var bridge: PrimeBridge
    @State private var showing = false

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            if showing {
                CashuScreen { showing = false }
                    .transition(.move(edge: .bottom))
            } else if wallet.busy == nil {
                // Hidden while the Fedimint wallet works, so the two never ask Prime at once.
                Button { showing = true } label: {
                    Label("Cashu", systemImage: "circle.hexagongrid.fill")
                        .font(.subheadline.weight(.semibold))
                        .padding(.horizontal, 18)
                        .frame(minHeight: 44)
                        .foregroundStyle(WalletStyle.background)
                        .background(WalletStyle.copper, in: Capsule())
                        .shadow(color: .black.opacity(0.4), radius: 8, y: 2)
                        .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .padding(.trailing, 20)
                .padding(.bottom, 8)
                .accessibilityLabel("Open Cashu")
                .accessibilityIdentifier("cashu.open")
            }
        }
        // Only the button and the screen take touches; the rest of the layer lets them through to
        // RootView. While the screen is closed the button stays put when a keyboard comes up.
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
        .ignoresSafeArea(showing ? [] : .keyboard, edges: .bottom)
        .animation(.easeOut(duration: 0.2), value: showing)
        .task { await cashu.open(bridge: bridge) }
    }
}
#endif
