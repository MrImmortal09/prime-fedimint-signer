import Combine
import SwiftUI

/// One request to Prime over QR codes: Prime scans the request off this screen, the person
/// approves on Prime, and this phone scans Prime's answer.
struct PrimeExchangeView: View {
    let exchange: PendingExchange
    @EnvironmentObject private var bridge: PrimeBridge
    @State private var frame = 0
    @State private var scanning = false
    @State private var progress = 0.0
    @State private var problem: String?
    @State private var scanner = AnswerScanner()
    private let ticker = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                if scanning {
                    answer
                } else {
                    request
                }
            }
            .padding()
            .navigationTitle("Passport Prime")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { bridge.cancel() }
                }
            }
        }
        .onReceive(ticker) { _ in
            if !scanning { frame += 1 }
        }
    }

    @ViewBuilder private var request: some View {
        Text("Scan this with Passport Prime").font(.headline)
        QRCodeView(text: exchange.frames[frame % exchange.frames.count])
            .frame(maxWidth: 340)
        Text(exchange.frames.count > 1 ? "Part \(frame % exchange.frames.count + 1) of \(exchange.frames.count)" : " ")
            .font(.caption)
            .foregroundStyle(.secondary)
        Text("Check the details on Prime and approve there. Then scan the answer Prime shows.")
            .multilineTextAlignment(.center)
            .font(.callout)
        Button("Scan Prime's answer") { scanning = true }
            .buttonStyle(.borderedProminent)
    }

    @ViewBuilder private var answer: some View {
        if Scanner.isAvailable {
            ScannerView { code in take(code) }
                .frame(height: 380)
                .clipShape(RoundedRectangle(cornerRadius: 12))
        } else {
            Text("This device cannot scan with its camera.")
                .multilineTextAlignment(.center)
        }
        ProgressView(value: progress)
        if let problem {
            Text(problem).font(.caption).foregroundStyle(.red)
        }
        Button("Show the request again") { scanning = false }
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
