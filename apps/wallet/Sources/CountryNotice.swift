#if os(macOS)
import SwiftUI

/// First-launch explanation of the default Mac-region country setting. The
/// presenting dashboard records acknowledgment in `onDone`, never when queued.
struct CountryNotice: View {
    let onDone: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Image(systemName: "globe")
                .font(.system(size: 30))
                .foregroundStyle(Color.aether)
            Text("Your country on the globe").font(.title3.bold())
            Text("Your country is shown on the globe; you can turn it off in Settings")
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Done", action: onDone)
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .padding(.top, 4)
        }
        .padding(24)
        .frame(width: 440)
        .interactiveDismissDisabled()
    }
}
#endif
