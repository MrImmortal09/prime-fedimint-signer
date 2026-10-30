import SwiftUI

/// Shared appearance for the phone companion. QR codes keep absolute black/white contrast.
enum WalletStyle {
    static let background = Color(red: 16/255, green: 17/255, blue: 19/255)
    static let surface = Color(red: 28/255, green: 30/255, blue: 32/255)
    static let ink = Color(red: 247/255, green: 245/255, blue: 240/255)
    static let muted = Color(red: 164/255, green: 167/255, blue: 172/255)
    static let copper = Color(red: 214/255, green: 154/255, blue: 120/255)
    static let mint = Color(red: 67/255, green: 217/255, blue: 163/255)
    static let danger = Color(red: 239/255, green: 114/255, blue: 114/255)
}

struct WalletButtonStyle: ButtonStyle {
    var secondary = false
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.headline).frame(maxWidth: .infinity, minHeight: 52)
            .foregroundStyle(secondary ? WalletStyle.ink : WalletStyle.background)
            .background(secondary ? WalletStyle.surface : WalletStyle.copper)
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .opacity(!enabled ? 0.4 : configuration.isPressed ? 0.75 : 1)
    }
}

struct WalletPanel<Content: View>: View {
    @ViewBuilder var content: Content
    var body: some View {
        VStack(alignment: .leading, spacing: 16) { content }
            .padding(20).frame(maxWidth: .infinity, alignment: .leading)
            .background(WalletStyle.surface, in: RoundedRectangle(cornerRadius: 24))
            .overlay(RoundedRectangle(cornerRadius: 24).stroke(WalletStyle.muted.opacity(0.18)))
    }
}

struct FlowStep: View {
    let number: Int
    let title: String
    let detail: String
    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            Text("\(number)").font(.headline).foregroundStyle(WalletStyle.copper)
                .frame(width: 32, height: 32).background(WalletStyle.copper.opacity(0.12), in: Circle())
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.headline)
                Text(detail).font(.subheadline).foregroundStyle(WalletStyle.muted)
            }
        }.accessibilityElement(children: .combine)
    }
}

struct WalletNotice: View {
    let title: String
    let message: String
    var symbol = "info.circle"
    var body: some View {
        Label { VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.headline)
            Text(message).font(.subheadline).foregroundStyle(WalletStyle.muted)
        } } icon: { Image(systemName: symbol).foregroundStyle(WalletStyle.copper) }
        .padding(.vertical, 8)
    }
}
