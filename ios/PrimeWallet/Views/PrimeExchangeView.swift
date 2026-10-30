import Combine
import SwiftUI

/// One request to Prime over QR codes: Prime scans the request off this screen, the person
/// approves on Prime, and this phone scans Prime's answer.
struct PrimeExchangeView: View {
    let exchange: PendingExchange
    @EnvironmentObject private var bridge: PrimeBridge
    /// How much each code carries: "low", "medium" or "high". Denser codes mean fewer parts.
    @AppStorage("qrDensity") private var densityName = "medium"
    @State private var frames: [String] = []
    @State private var frame = 0
    @State private var scanning = false
    @State private var progress = 0.0
    @State private var problem: String?
    @State private var scanner = AnswerScanner()
    /// The camera keeps reporting codes while the screen closes; the answer is taken once.
    @State private var done = false
    private let ticker = Timer.publish(every: 0.25, on: .main, in: .common).autoconnect()

    private var density: QrDensity {
        switch densityName {
        case "low": return .low
        case "high": return .high
        default: return .medium
        }
    }

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
        .onAppear { reframe() }
        .onChange(of: densityName) { _ in reframe() }
        .onReceive(ticker) { _ in
            if !scanning { frame += 1 }
        }
    }

    private func reframe() {
        frames = requestFrames(request: exchange.request, density: density)
        frame = 0
    }

    @ViewBuilder private var request: some View {
        Text("Scan this with Passport Prime").font(.headline)
        if frames.isEmpty {
            ProgressView()
        } else {
            QRCodeView(text: frames[frame % frames.count])
                .frame(maxWidth: 340)
            Text(frames.count > 1 ? "Part \(frame % frames.count + 1) of \(frames.count)" : " ")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        Picker("Code size", selection: $densityName) {
            Text("Small").tag("low")
            Text("Medium").tag("medium")
            Text("Large").tag("high")
        }
        .pickerStyle(.segmented)
        Text("Larger codes mean fewer parts. Choose smaller ones if Prime cannot read them.")
            .font(.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
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
        guard !done else { return }
        switch scanner.receive(code: code) {
        case .partial(let progress):
            self.progress = progress
            problem = nil
        case .complete(let answer):
            done = true
            progress = 1
            bridge.answer(answer)
        case .rejected(let reason):
            problem = reason
        }
    }
}
