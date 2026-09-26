import Charts
import CoreImage.CIFilterBuiltins
import SwiftUI

/// Everyday wallet, modeled on Phantom (centered balance, round actions, token
/// list) and IPFS Desktop (sidebar, "connected" status page with stat tiles and
/// traffic chart). macOS: sidebar; iOS: bottom tabs. Proofs, roots and raw logs
/// live in Developer mode.
struct SimpleDashboard: View {
    @EnvironmentObject var model: WalletModel
    @State private var page: Page? = .home
    @State private var sheet: Sheet?

    enum Page: String, CaseIterable, Identifiable {
        case home = "Home", activity = "Activity", network = "Network", security = "Security"
        var id: String { rawValue }
        var icon: String {
            switch self {
            case .home: "house.fill"
            case .activity: "clock.arrow.circlepath"
            case .network: "point.3.connected.trianglepath.dotted"
            case .security: "lock.shield.fill"
            }
        }
    }

    enum Sheet: String, Identifiable {
        case send, receive
        var id: String { rawValue }
    }

    var body: some View {
        shell
            .tint(.aether)
            .sheet(item: $sheet) { s in
                switch s {
                case .send: SendSheet()
                case .receive: ReceiveSheet()
                }
            }
    }

    #if os(macOS)
    private var shell: some View {
        NavigationSplitView {
            List(Page.allCases, selection: $page) { p in
                Label(p.rawValue, systemImage: p.icon).tag(p)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 190)
            .safeAreaInset(edge: .bottom) { SidebarStatus().padding(12) }
        } detail: {
            ScrollView {
                pageView(page ?? .home).padding(28).frame(maxWidth: 820).frame(maxWidth: .infinity)
            }
            .navigationTitle(page?.rawValue ?? "Home")
        }
        .frame(minWidth: 900, minHeight: 660)
    }
    #else
    private var shell: some View {
        TabView(selection: Binding(get: { page ?? .home }, set: { page = $0 })) {
            ForEach(Page.allCases) { p in
                NavigationStack {
                    ScrollView { pageView(p).padding(16) }.navigationTitle(p == .home ? "" : p.rawValue)
                }
                .tabItem { Label(p.rawValue, systemImage: p.icon) }
                .tag(p)
            }
        }
    }
    #endif

    @ViewBuilder private func pageView(_ p: Page) -> some View {
        switch p {
        case .home: HomePage(sheet: $sheet) { page = .activity }
        case .activity: ActivityPage()
        case .network: NetworkPage()
        case .security: SecurityPage()
        }
    }
}

extension Color {
    /// Aether accent: a violet in the family of Phantom's, readable in light and dark.
    static let aether = Color(red: 0.49, green: 0.40, blue: 0.95)
}

// MARK: - Pages

private struct HomePage: View {
    @EnvironmentObject var model: WalletModel
    @Binding var sheet: SimpleDashboard.Sheet?
    let showActivity: () -> Void

    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }

    var body: some View {
        VStack(spacing: 22) {
            hero
            HStack(spacing: 28) {
                RoundAction(title: "Receive", icon: "qrcode") { sheet = .receive }.disabled(model.address.isEmpty)
                RoundAction(title: "Send", icon: "paperplane.fill") { sheet = .send }.disabled(model.busy || model.account == nil)
                RoundAction(title: "Get AETH", icon: "drop.fill") { model.faucet() }.disabled(model.busy || model.address.isEmpty)
            }
            BalanceCard()
            Card {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Tokens").font(.headline)
                    TokenRow(symbol: "AETH", name: "Aether", amount: model.account == nil ? nil : balance, verified: model.verifyError == nil && model.account != nil)
                }
            }
            Card {
                VStack(alignment: .leading, spacing: 10) {
                    HStack {
                        Text("Recent activity").font(.headline)
                        Spacer()
                        if !model.activity.isEmpty { Button("See all", action: showActivity).buttonStyle(.borderless) }
                    }
                    ActivityList(limit: 3)
                }
            }
        }
    }

    private var hero: some View {
        VStack(spacing: 8) {
            Button {
                Clipboard.copy(model.address)
            } label: {
                HStack(spacing: 6) {
                    Circle().fill(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing)).frame(width: 20, height: 20)
                    Text("Account 1").font(.callout.weight(.semibold))
                    Text(Short.address(model.address)).font(.callout.monospaced()).foregroundStyle(.secondary)
                    Image(systemName: "doc.on.doc").font(.caption).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12).padding(.vertical, 6)
                .background(.background.secondary, in: Capsule())
            }
            .buttonStyle(.plain)
            .help("Copy address")
            Text(model.account == nil ? "—" : "\(Amount.text(balance)) AETH")
                .font(.system(size: 52, weight: .bold, design: .rounded))
                .contentTransition(.numericText())
            VerifiedBadge()
        }
        .padding(.top, 8)
    }
}

private struct ActivityPage: View {
    var body: some View {
        Card { ActivityList(limit: 100) }
    }
}

private struct NetworkPage: View {
    @EnvironmentObject var model: WalletModel

    private var blockTime: String {
        let b = model.blocks.sorted { $0.height < $1.height }
        guard b.count > 1, let first = b.first, let last = b.last, last.timestampMs > first.timestampMs else { return "—" }
        let s = Double(last.timestampMs - first.timestampMs) / 1000 / Double(b.count - 1)
        return String(format: "%.1f s", s)
    }

    var body: some View {
        VStack(spacing: 16) {
            Card {
                HStack(spacing: 16) {
                    Image(systemName: model.status == nil ? "antenna.radiowaves.left.and.right.slash" : "checkmark.circle.fill")
                        .font(.system(size: 40)).foregroundStyle(model.status == nil ? Color.orange : Color.green)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(model.status == nil ? "Connecting to Aether…" : "Connected to Aether").font(.title2.bold())
                        Text("Found the validators on the public DHT. Your balance is checked on this device against their group signature.")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                }
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 12)], spacing: 12) {
                Tile(value: model.status.map { "#\($0.height)" } ?? "—", label: "Latest block", icon: "cube")
                Tile(value: "\(model.validators)", label: "Validators", icon: "person.3.fill")
                Tile(value: blockTime, label: "Block time", icon: "timer")
                Tile(value: model.status.map { Amount.fee($0.transferFeeWei) } ?? "—", label: "Transfer fee", icon: "flame")
                Tile(value: model.status.map { "\($0.mempool)" } ?? "—", label: "Waiting txs", icon: "tray.full")
            }
            NetworkCard()
        }
    }
}

private struct SecurityPage: View {
    var body: some View {
        VStack(spacing: 16) {
            Card {
                HStack(alignment: .top, spacing: 14) {
                    Image(systemName: "lock.shield.fill").font(.system(size: 34)).foregroundStyle(Color.aether)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Protected by this device").font(.title3.bold())
                        Text("Your key was created inside the Secure Enclave and can never be copied out. Every payment asks for Touch ID or your password. There is no seed phrase to lose.")
                            .font(.callout).foregroundStyle(.secondary)
                    }
                }
            }
            Card { RecoveryPanel() }
        }
    }
}

// MARK: - Sidebar status (IPFS Desktop style)

private struct SidebarStatus: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        HStack(spacing: 8) {
            Circle().fill(model.status == nil ? Color.orange : Color.green).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                Text(model.status == nil ? "Connecting" : "Connected").font(.caption.weight(.semibold))
                Text(model.status.map { "Block #\($0.height)" } ?? "Searching DHT…").font(.caption2).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }
}

// MARK: - Cards

private struct Card<Content: View>: View {
    let content: Content
    init(@ViewBuilder _ content: () -> Content) { self.content = content() }

    var body: some View {
        content
            .padding(18)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

private struct BalanceCard: View {
    @EnvironmentObject var model: WalletModel
    @State private var range: Range = .day

    enum Range: String, CaseIterable, Identifiable {
        case hour = "1H", day = "1D", week = "1W", all = "All"
        var id: String { rawValue }
        var seconds: TimeInterval? {
            switch self {
            case .hour: 3_600
            case .day: 86_400
            case .week: 604_800
            case .all: nil
            }
        }
    }

    private var points: [BalancePoint] {
        guard let s = range.seconds else { return model.history }
        let from = Date().addingTimeInterval(-s)
        let inRange = model.history.filter { $0.date >= from }
        // Carry the last earlier value in so the line starts at the left edge.
        var pts = inRange
        if let before = model.history.last(where: { $0.date < from }) {
            pts.insert(BalancePoint(date: from, aeth: before.aeth), at: 0)
        }
        // Extend the last known balance to now, so even one observation draws a line.
        if let last = pts.last, Date().timeIntervalSince(last.date) > 1 {
            pts.append(BalancePoint(date: Date(), aeth: last.aeth))
        }
        return pts
    }

    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    change
                    Spacer()
                    Picker("Range", selection: $range) {
                        ForEach(Range.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .fixedSize()
                }
                chart.frame(height: 170)
            }
        }
    }

    @ViewBuilder private var change: some View {
        if let first = points.first, points.count > 1 {
            let d = balance - first.aeth
            Label("\(d >= 0 ? "+" : "")\(Amount.text(d)) AETH in \(range == .all ? "total" : range.rawValue)",
                  systemImage: d >= 0 ? "arrow.up.right" : "arrow.down.right")
                .font(.callout.weight(.medium))
                .foregroundStyle(d >= 0 ? .green : .red)
        } else {
            Text("Your balance history appears here as it changes.").font(.callout).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var chart: some View {
        if points.count > 1 {
            Chart(points) { p in
                AreaMark(x: .value("Time", p.date), y: .value("AETH", p.aeth))
                    .interpolationMethod(.stepEnd)
                    .foregroundStyle(.linearGradient(colors: [.aether.opacity(0.35), .aether.opacity(0.02)], startPoint: .top, endPoint: .bottom))
                LineMark(x: .value("Time", p.date), y: .value("AETH", p.aeth))
                    .interpolationMethod(.stepEnd)
                    .lineStyle(StrokeStyle(lineWidth: 2.5))
                    .foregroundStyle(Color.aether)
            }
            .chartYScale(domain: .automatic(includesZero: true))
            .chartXAxis { AxisMarks(values: .automatic(desiredCount: 4)) }
        } else {
            RoundedRectangle(cornerRadius: 10).fill(.quaternary.opacity(0.5))
                .overlay(Image(systemName: "chart.xyaxis.line").font(.largeTitle).foregroundStyle(.tertiary))
        }
    }
}

private struct VerifiedBadge: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        if model.account != nil && model.verifyError == nil {
            Label("Verified", systemImage: "checkmark.shield.fill")
                .font(.caption.weight(.semibold)).foregroundStyle(.green)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(.green.opacity(0.12), in: Capsule())
                .help("This device checked the balance itself against the validators' signature. No server was trusted.")
        } else {
            Label(model.status == nil ? "Connecting" : "Checking", systemImage: "hourglass")
                .font(.caption.weight(.semibold)).foregroundStyle(.orange)
                .padding(.horizontal, 10).padding(.vertical, 4)
                .background(.orange.opacity(0.12), in: Capsule())
        }
    }
}

private struct NetworkCard: View {
    @EnvironmentObject var model: WalletModel

    private var blocks: [BlockInfo] { model.blocks.sorted { $0.height < $1.height } }

    var body: some View {
        Card {
            VStack(alignment: .leading, spacing: 12) {
                Text("Activity on the network").font(.headline)
                if !blocks.isEmpty {
                    let busiest = blocks.map(\.txs).max() ?? 0
                    Text("Transactions in the last \(blocks.count) blocks").font(.caption).foregroundStyle(.secondary)
                    Chart(blocks, id: \.height) { b in
                        // Empty blocks show as a short stub so the rhythm of blocks stays visible.
                        BarMark(x: .value("Block", String(b.height)), y: .value("Txs", b.txs > 0 ? Double(b.txs) : Double(max(busiest, 1)) * 0.04))
                            .foregroundStyle(b.txs > 0 ? Color.aether : Color.secondary.opacity(0.35))
                            .cornerRadius(2)
                    }
                    .chartXAxis(.hidden)
                    .chartYScale(domain: 0...Double(max(busiest, 1)))
                    .chartYAxis { AxisMarks(values: .automatic(desiredCount: 3)) }
                    .frame(height: 140)
                    if busiest == 0 {
                        Text("Quiet: no transactions right now.").font(.caption2).foregroundStyle(.tertiary)
                    }
                }
            }
        }
    }
}

private struct ActivityRow: View {
    let item: ActivityItem

    private var icon: (String, Color) {
        switch item.kind {
        case .sent: ("arrow.up.right.circle.fill", .blue)
        case .received: ("arrow.down.left.circle.fill", .green)
        case .security: ("lock.shield.fill", .purple)
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon.0).font(.title2).foregroundStyle(icon.1)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title).font(.callout.weight(.medium))
                Text(item.date, style: .relative).font(.caption).foregroundStyle(.secondary)
                    + Text(" ago").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 2) {
                if let a = item.amount {
                    Text("\(a >= 0 ? "+" : "")\(Amount.text(a))").font(.callout.weight(.semibold).monospacedDigit())
                        .foregroundStyle(a >= 0 ? .green : .primary)
                }
                switch item.state {
                case .pending: Text("Confirming…").font(.caption).foregroundStyle(.orange)
                case .done: Text("Done").font(.caption).foregroundStyle(.secondary)
                case .failed: Text("Failed").font(.caption).foregroundStyle(.red)
                }
            }
        }
    }
}


private struct RoundAction: View {
    let title: String
    let icon: String
    let action: () -> Void
    @Environment(\.isEnabled) private var enabled

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: icon).font(.title3.weight(.semibold))
                    .frame(width: 54, height: 54)
                    .background(Color.aether.opacity(0.14), in: Circle())
                    .foregroundStyle(Color.aether)
                Text(title).font(.caption.weight(.medium)).foregroundStyle(.primary)
            }
            .opacity(enabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
    }
}

private struct TokenRow: View {
    let symbol: String
    let name: String
    let amount: Double?
    let verified: Bool

    var body: some View {
        HStack(spacing: 12) {
            Text("Æ").font(.headline.bold()).foregroundStyle(.white)
                .frame(width: 40, height: 40)
                .background(LinearGradient(colors: [.aether, .pink], startPoint: .topLeading, endPoint: .bottomTrailing), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.callout.weight(.semibold))
                HStack(spacing: 4) {
                    Text(amount.map { "\(Amount.text($0)) \(symbol)" } ?? "—").font(.caption).foregroundStyle(.secondary)
                    if verified { Image(systemName: "checkmark.seal.fill").font(.caption2).foregroundStyle(.green) }
                }
            }
            Spacer()
            Text(amount.map(Amount.text) ?? "—").font(.callout.weight(.semibold).monospacedDigit())
        }
    }
}

private struct Tile: View {
    let value: String
    let label: String
    let icon: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Image(systemName: icon).foregroundStyle(Color.aether)
            Text(value).font(.title3.weight(.semibold).monospacedDigit()).lineLimit(1).minimumScaleFactor(0.6)
            Text(label).font(.caption).foregroundStyle(.secondary)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
    }
}

private struct ActivityList: View {
    @EnvironmentObject var model: WalletModel
    let limit: Int

    var body: some View {
        let items = Array(model.activity.prefix(limit))
        if items.isEmpty {
            Text("Nothing yet. Tap Get AETH to receive test tokens.").font(.callout).foregroundStyle(.secondary)
        } else {
            VStack(spacing: 10) {
                ForEach(items) { item in
                    ActivityRow(item: item)
                    if item.id != items.last?.id { Divider() }
                }
            }
        }
    }
}

// MARK: - Sheets

private struct SendSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss

    private var amount: Double? { Double(model.sendAmount) }
    private var balance: Double { model.account.flatMap { Double(Wei.format($0.balanceWei)) } ?? 0 }
    private var valid: Bool { !model.sendTo.isEmpty && (amount ?? 0) > 0 && (amount ?? 0) <= balance }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Send AETH").font(.title2.bold())
            VStack(alignment: .leading, spacing: 6) {
                Text("To").font(.caption).foregroundStyle(.secondary)
                TextField("0x… (several: separate with commas)", text: $model.sendTo)
                    .textFieldStyle(.roundedBorder).font(.body.monospaced())
            }
            VStack(alignment: .leading, spacing: 6) {
                Text("Amount (each)").font(.caption).foregroundStyle(.secondary)
                HStack {
                    TextField("0", text: $model.sendAmount).textFieldStyle(.roundedBorder).font(.title3.monospacedDigit())
                    Text("AETH").foregroundStyle(.secondary)
                    Button("Max") { model.sendAmount = Amount.text(max(0, balance - 0.001)) }.buttonStyle(.borderless)
                }
            }
            HStack {
                Text("Available").foregroundStyle(.secondary)
                Spacer()
                Text("\(Amount.text(balance)) AETH").monospacedDigit()
            }.font(.callout)
            if let s = model.status {
                HStack {
                    Text("Network fee").foregroundStyle(.secondary)
                    Spacer()
                    Text("≈ \(Amount.fee(s.transferFeeWei))").monospacedDigit()
                }.font(.callout)
            }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button {
                    model.send()
                    dismiss()
                } label: { Label("Send", systemImage: "touchid").frame(minWidth: 100) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!valid || model.busy)
            }
        }
        .padding(24)
        .frame(minWidth: 420)
    }
}

private struct ReceiveSheet: View {
    @EnvironmentObject var model: WalletModel
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false

    var body: some View {
        VStack(spacing: 16) {
            Text("Receive AETH").font(.title2.bold())
            QRCode(text: model.address).frame(width: 200, height: 200)
            Text(model.address).font(.callout.monospaced()).multilineTextAlignment(.center).textSelection(.enabled)
            HStack {
                Button {
                    Clipboard.copy(model.address)
                    copied = true
                } label: { Label(copied ? "Copied" : "Copy address", systemImage: copied ? "checkmark" : "doc.on.doc") }
                    .buttonStyle(.borderedProminent)
                Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(24)
        .frame(minWidth: 380)
    }
}

private struct RecoveryPanel: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Recovery device").font(.title3.bold())
            Text("If you lose this device, a second device you trust (your other Mac or iPhone) can move your funds to itself.")
                .font(.callout).foregroundStyle(.secondary)
            step(1, "On the other device, copy its code", "Open Aether there, tap Recovery device, and copy \"This device's code\".")
            HStack {
                TextField("Paste the other device's code", text: $model.guardianInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                Button("Trust it") { model.setRecoveryKey() }.buttonStyle(.borderedProminent).disabled(model.busy || model.guardianInput.isEmpty)
            }
            Divider()
            step(2, "This device's code", "Give it to someone who wants this device as their recovery device.")
            HStack {
                Text(model.recoveryCode.isEmpty ? "…" : "\(model.recoveryCode.prefix(24))…").font(.caption.monospaced())
                Spacer()
                Button { Clipboard.copy(model.recoveryCode) } label: { Label("Copy", systemImage: "doc.on.doc") }.disabled(model.recoveryCode.isEmpty)
            }
            Divider()
            step(3, "Recover a lost account", "Only works if that account trusted this device.")
            HStack {
                TextField("0x lost account", text: $model.lostInput).textFieldStyle(.roundedBorder).font(.caption.monospaced())
                Button("Recover") { model.recover() }.disabled(model.busy || model.lostInput.isEmpty)
            }
        }
    }

    private func step(_ n: Int, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text("\(n)").font(.caption.bold()).frame(width: 22, height: 22).background(Color.aether.opacity(0.15), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.callout.weight(.semibold))
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

private struct QRCode: View {
    let text: String

    var body: some View {
        if let img = Self.render(text) {
            Image(decorative: img, scale: 1).interpolation(.none).resizable().scaledToFit()
        } else {
            Image(systemName: "qrcode").resizable().scaledToFit().foregroundStyle(.tertiary)
        }
    }

    static func render(_ s: String) -> CGImage? {
        guard !s.isEmpty else { return nil }
        let f = CIFilter.qrCodeGenerator()
        f.message = Data(s.utf8)
        f.correctionLevel = "M"
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else { return nil }
        return CIContext().createCGImage(out, from: out.extent)
    }
}

// MARK: - Formatting

enum Amount {
    static func text(_ v: Double) -> String {
        v.formatted(.number.precision(.fractionLength(0...4)))
    }

    /// Fee in AETH with enough digits to be non-zero.
    static func fee(_ wei: String) -> String {
        let aeth = (Double(wei) ?? 0) / 1e18
        if aeth == 0 { return "free" }
        return "\(aeth.formatted(.number.precision(.significantDigits(1...3)))) AETH"
    }
}
