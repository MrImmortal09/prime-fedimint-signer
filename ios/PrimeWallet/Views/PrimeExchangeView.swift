import Combine
import SwiftUI

/// One request to Prime over QR codes: Prime scans the request off this screen, the person
/// approves on Prime, and this phone scans Prime's answer.
struct PrimeExchangeView: View {
    let exchange: PendingExchange
    @EnvironmentObject private var bridge: PrimeBridge
    @State private var frame = 0
    @State private var scanning = false
    @State private var paused = false
    @State private var progress = 0.0
    @State private var problem: String?
    @State private var scanner = AnswerScanner()
    private let ticker = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 24) {
                    HStack {
                        Text(scanning ? "STEP 2 OF 2" : "STEP 1 OF 2").font(.caption.weight(.semibold)).tracking(2)
                        Spacer()
                        Label("QR connection", systemImage: "qrcode").font(.caption)
                    }.foregroundStyle(WalletStyle.copper)
                    if scanning { answer } else { request }
                }.padding(24).frame(maxWidth: 600)
            }.background(WalletStyle.background)
            .navigationTitle("Passport Prime")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { bridge.cancel() }
                }
            }
        }
        .onReceive(ticker) { _ in
            if !scanning && !paused { frame += 1 }
        }
    }

    @ViewBuilder private var request: some View {
        Text("Review on your Passport").font(.largeTitle.bold()).frame(maxWidth: .infinity, alignment: .leading)
        QRCodeView(text: exchange.frames[frame % exchange.frames.count])
            .padding(16).background(.white, in: RoundedRectangle(cornerRadius: 20))
            .frame(maxWidth: 340)
            .accessibilityLabel("Animated authorization request for Passport Prime")
        Text(exchange.frames.count > 1 ? "Part \(frame % exchange.frames.count + 1) of \(exchange.frames.count)" : " ")
            .font(.caption)
            .foregroundStyle(.secondary)
        if exchange.frames.count > 1 {
            Button(paused ? "Resume QR animation" : "Pause QR animation") { paused.toggle() }
                .font(.subheadline).frame(minHeight: 44)
        }
        Text("On Prime, open Fedimint and tap Scan Request. Check the amount, destination, fees, and warnings. Slide to approve only if everything matches.")
            .multilineTextAlignment(.center)
            .font(.callout)
        Button("Scan Passport response") { scanning = true }
            .buttonStyle(WalletButtonStyle())
        Text("Keep this screen open while Prime scans. If scanning stalls, keep both screens steady and adjust the distance.")
            .font(.footnote).foregroundStyle(WalletStyle.muted)
    }

    @ViewBuilder private var answer: some View {
        Text("Bring the answer home").font(.largeTitle.bold()).frame(maxWidth: .infinity, alignment: .leading)
        Text("Scan the response shown on Prime. Your phone will verify it and continue the operation.").foregroundStyle(WalletStyle.muted)
        if Scanner.isAvailable {
            ScannerView { code in take(code) }
                .frame(height: 380)
                .clipShape(RoundedRectangle(cornerRadius: 12))
        } else {
            Text("This device cannot scan with its camera.")
                .multilineTextAlignment(.center)
        }
        ProgressView(value: progress).tint(WalletStyle.copper)
        Text(progress > 0 ? "Reading response · \(Int(progress * 100))%" : "Waiting for the response QR")
            .font(.subheadline).foregroundStyle(WalletStyle.muted)
        if let problem {
            Text(problem).font(.caption).foregroundStyle(.red)
        }
        Button("Show request again") { scanning = false }.buttonStyle(WalletButtonStyle(secondary: true))
    }

    private func take(_ code: String) {
        switch scanner.receive(code: code) {
        case .partial(let progress):
            self.progress = progress
            problem = nil
        case .complete(let answer):
            bridge.answer(answer)
        case .rejected(let reason):
            problem = reason
        }
    }
}
