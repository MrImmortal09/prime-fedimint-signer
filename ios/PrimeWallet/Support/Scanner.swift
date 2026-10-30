import AVFoundation
import SwiftUI

enum Scanner {
    /// Whether this device has a camera to scan with. The iOS Simulator does not.
    static var isAvailable: Bool {
        AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back) != nil
    }
}

/// The camera, reporting every QR code in every frame it sees, so the parts of an animated code
/// arrive as fast as the other screen shows them.
///
/// VisionKit's scanner is not used: it tracks a code once found and is slow to notice the same
/// spot showing a different one, so an animated code read through it stalls on its first part.
struct ScannerView: UIViewRepresentable {
    let onCode: (String) -> Void

    func makeUIView(context: Context) -> CameraPreview {
        let preview = CameraPreview()
        context.coordinator.onCode = onCode
        context.coordinator.attach(to: preview)
        return preview
    }

    func updateUIView(_ preview: CameraPreview, context: Context) {
        context.coordinator.onCode = onCode
    }

    static func dismantleUIView(_ preview: CameraPreview, coordinator: Coordinator) {
        coordinator.stop()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    final class Coordinator: NSObject, AVCaptureMetadataOutputObjectsDelegate {
        var onCode: ((String) -> Void)?
        private let session = AVCaptureSession()
        private let queue = DispatchQueue(label: "dev.primefedimint.wallet.scanner")

        func attach(to preview: CameraPreview) {
            preview.previewLayer.session = session
            preview.previewLayer.videoGravity = .resizeAspectFill
            // Returns at once when the person has already answered the camera prompt.
            AVCaptureDevice.requestAccess(for: .video) { [self] granted in
                guard granted else { return }
                queue.async { [self] in
                    configure()
                    session.startRunning()
                }
            }
        }

        func stop() {
            queue.async { [session] in
                if session.isRunning { session.stopRunning() }
            }
        }

        private func configure() {
            guard session.inputs.isEmpty,
                  let camera = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
                  let input = try? AVCaptureDeviceInput(device: camera)
            else { return }
            session.beginConfiguration()
            defer { session.commitConfiguration() }
            guard session.canAddInput(input) else { return }
            session.addInput(input)
            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else { return }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: queue)
            output.metadataObjectTypes = [.qr]
            // A screen is held close to the camera.
            if (try? camera.lockForConfiguration()) != nil {
                if camera.isAutoFocusRangeRestrictionSupported {
                    camera.autoFocusRangeRestriction = .near
                }
                camera.unlockForConfiguration()
            }
        }

        func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
            let codes = objects.compactMap { ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue }
            guard !codes.isEmpty else { return }
            DispatchQueue.main.async { [weak self] in
                guard let onCode = self?.onCode else { return }
                codes.forEach(onCode)
            }
        }
    }
}

/// A view whose layer shows the camera.
final class CameraPreview: UIView {
    override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }

    var previewLayer: AVCaptureVideoPreviewLayer {
        layer as! AVCaptureVideoPreviewLayer
    }
}

/// A sheet that scans one code, such as an invite code, an invoice or a token.
struct ScanSheet: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    /// The camera keeps reporting the code while the sheet closes; only the first read counts.
    @State private var done = false

    var body: some View {
        NavigationStack {
            ScannerView { code in
                guard !done else { return }
                done = true
                onCode(code)
                dismiss()
            }
            .ignoresSafeArea()
            .navigationTitle("Scan")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
    }
}
