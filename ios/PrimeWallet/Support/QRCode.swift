import CoreImage.CIFilterBuiltins
import SwiftUI

enum QRCode {
    private static let context = CIContext()

    static func image(for text: String, correctionLevel: String = "L") -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = correctionLevel
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
        guard let image = context.createCGImage(scaled, from: scaled.extent) else { return nil }
        return UIImage(cgImage: image)
    }
}

/// A QR code on a white card, so it scans from any background.
struct QRCodeView: View {
    let text: String
    var correctionLevel = "L"

    var body: some View {
        Group {
            if let image = QRCode.image(for: text, correctionLevel: correctionLevel) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
            } else {
                Text("This is too long for one QR code.")
            }
        }
        .padding(12)
        .background(Color.white)
        .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}
