import Combine
import SwiftUI

/// One request to Prime over QR codes: Prime scans the request off this screen, the person
/// approves on Prime, and this phone scans Prime's answer.
struct PrimeExchangeView: View {
    let exchange: PendingExchange
    @EnvironmentObject private var bridge: PrimeBridge
    /// How much each code carries: "low", "medium" or "high". Denser codes mean fewer parts.
    @AppStorage("qrDensity") private var densityName = "low"
    @State private var frames: [String] = []
    @State private var frame = 0
    @State private var scanning = false
    @State private var paused = false
    @State private var progress = 0.0
    @State private var problem: String?
    @State private var scanner = AnswerScanner()
    /// The camera keeps reporting codes while the screen closes; the answer is taken once.
    @State private var done = false
    private let ticker = Timer.publish(every: 0.8, on: .main, in: .common).autoconnect()

    private var density: QrDensity {
        switch densityName {
        case "low": return .low
        case "high": return .high
        default: return .medium
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 12) {
                    HStack {
                        Text(exchange.pairingStep.map { "PAIRING · \($0) OF 2" } ?? (scanning ? "SCAN RESPONSE" : "SHOW REQUEST")).font(.caption.weight(.semibold)).tracking(2)
                        Spacer()
                        Label("QR connection", systemImage: "qrcode").font(.caption)
                    }.foregroundStyle(WalletStyle.copper)
                    Text(exchange.title).font(.title2.bold())
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if scanning { answer } else { request }
                }.padding(20).frame(maxWidth: 600)
            }.background(WalletStyle.background)
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
            if !scanning && !paused && !frames.isEmpty { frame = (frame + 1) % frames.count }
        }
    }

    private func reframe() {
        frames = requestFrames(request: exchange.request, density: density)
        frame = 0
    }

    @ViewBuilder private var request: some View {
        if frames.isEmpty {
            ProgressView()
        } else {
            QRCodeView(text: frames[frame % frames.count], correctionLevel: "M")
                .padding(16).background(.white, in: RoundedRectangle(cornerRadius: 20))
                .frame(maxWidth: 340)
                .accessibilityLabel("Animated authorization request for Passport Prime")
            HStack {
                Text(frames.count > 1 ? "Part \(frame % frames.count + 1) of \(frames.count)" : "Ready to scan")
                    .font(.caption).foregroundStyle(WalletStyle.muted)
                Spacer()
                if frames.count > 1 {
                    Button(paused ? "Resume" : "Pause") { paused.toggle() }
                        .font(.subheadline).frame(minWidth: 60, minHeight: 44)
                        .accessibilityLabel(paused ? "Resume QR animation" : "Pause QR animation")
                }
            }
            if paused && frames.count > 1 {
                HStack {
                    Button {
                        frame = (frame + frames.count - 1) % frames.count
                    } label: {
                        Label("Previous", systemImage: "chevron.left").frame(minHeight: 44)
                    }
                    Spacer()
                    Button {
                        frame = (frame + 1) % frames.count
                    } label: {
                        Label("Next", systemImage: "chevron.right").frame(minHeight: 44)
                    }
                }
            }

        }
        Picker("Code size", selection: $densityName) {
            Text("Small").tag("low")
            Text("Medium").tag("medium")
            Text("Large").tag("high")
        }
        .pickerStyle(.segmented)
        Text(exchange.instructions)
            .multilineTextAlignment(.center)
            .font(.callout)
        Button("Scan Passport response") { scanning = true }
            .buttonStyle(WalletButtonStyle())
    }

    @ViewBuilder private var answer: some View {
        Text(exchange.pairingStep == 1
             ? "Scan Prime’s response. Next, a second QR will appear to connect this wallet."
             : exchange.pairingStep == 2
             ? "Scan Prime’s response to finish pairing and open your wallet."
             : "Scan the response shown on Prime to continue.").foregroundStyle(WalletStyle.muted)
        if Scanner.isAvailable {
            ScannerView { code in take(code) }
                .frame(height: 280)
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
        guard !done else { return }
        switch scanner.receive(code: code) {
        case .partial(let progress):
            self.progress = progress
            problem = nil
        case .complete(let answer):
            // fps-v1 encodes its fixed header as {v: 1, id: bytes(16), body: ...}.
            // Reject a stale QR before resuming the wallet continuation. Rust still
            // performs the authoritative full envelope and request-id validation.
            let header = Data([0xa3, 0x61, 0x76, 0x01, 0x62, 0x69, 0x64, 0x50])
            guard exchange.request.starts(with: header), answer.starts(with: header),
                  exchange.request.count >= 24, answer.count >= 24,
                  exchange.request.prefix(24) == answer.prefix(24) else {
                scanner = AnswerScanner()
                progress = 0
                problem = "This response is for a different request. On Prime, tap Scan next request and scan the current code from this phone."
                return
            }
            done = true
            progress = 1
            bridge.answer(answer)
        case .rejected(let reason):
            scanner = AnswerScanner()
            progress = 0
            problem = reason
        }
    }
}
