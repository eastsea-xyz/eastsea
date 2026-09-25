import SwiftUI

struct ContentView: View {
    @EnvironmentObject var model: WalletModel

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 14) {
                header
                accountCard
                sendCard
                activity
            }
            .frame(minWidth: 460)
            blocksPanel.frame(width: 300)
        }
        .padding(20)
        .frame(minWidth: 800, minHeight: 620)
        .onAppear { model.start() }
    }

    private var header: some View {
        HStack {
            Image(systemName: "cube.transparent").font(.title)
            VStack(alignment: .leading) {
                Text("Aether Wallet").font(.title2.bold())
                if let s = model.status {
                    Text("devnet \(s.chainId) · height \(s.height) · \(model.validators) validators")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("connecting…").font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
            TextField("RPC", text: $model.rpc).textFieldStyle(.roundedBorder).frame(width: 190).font(.caption.monospaced())
        }
    }

    private var accountCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(model.address.isEmpty ? "—" : model.address).font(.callout.monospaced()).textSelection(.enabled)
                    Button { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(model.address, forType: .string) }
                        label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless)
                }
                Text(model.account.map { "\(Wei.format($0.balanceWei)) AETH" } ?? "…")
                    .font(.system(size: 34, weight: .semibold, design: .rounded))
                if let a = model.account, model.verifyError == nil {
                    Label("Verified by my Mac", systemImage: "checkmark.seal.fill").foregroundStyle(.green).font(.headline)
                    Text("Finality certificate of block \(a.certifiedBlock) checked against \(a.validators) validator keys · state root \(a.stateRoot.prefix(12))… · EIP-7864 proof for this address")
                        .font(.caption).foregroundStyle(.secondary)
                } else if let e = model.verifyError {
                    Label("Not verified", systemImage: "exclamationmark.triangle.fill").foregroundStyle(.orange).font(.headline)
                    Text(e).font(.caption).foregroundStyle(.secondary).lineLimit(3)
                }
                HStack {
                    Label("Key in Secure Enclave", systemImage: "lock.shield").font(.caption)
                    Spacer()
                    Button("Get 10 test AETH") { model.faucet() }.disabled(model.busy || model.address.isEmpty)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        } label: { Text("Account") }
    }

    private var sendCard: some View {
        GroupBox {
            HStack {
                TextField("0x recipient", text: $model.sendTo).textFieldStyle(.roundedBorder).font(.callout.monospaced())
                TextField("AETH", text: $model.sendAmount).textFieldStyle(.roundedBorder).frame(width: 80)
                Button("Send") { model.send() }.keyboardShortcut(.return).disabled(model.busy || model.sendTo.isEmpty)
            }
        } label: { Text("Send (signed in the Secure Enclave)") }
    }

    private var activity: some View {
        GroupBox {
            ScrollView {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(Array(model.log.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.caption.monospaced()).frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }.frame(minHeight: 120)
        } label: { Text("Activity") }
    }

    private var blocksPanel: some View {
        GroupBox {
            List(model.blocks, id: \.height) { b in
                VStack(alignment: .leading, spacing: 2) {
                    HStack {
                        Text("#\(b.height)").font(.callout.bold().monospacedDigit())
                        Spacer()
                        Text("\(b.txs) tx").font(.caption).foregroundStyle(b.txs > 0 ? .primary : .secondary)
                    }
                    Text("root \(b.stateRoot.prefix(18))…").font(.caption2.monospaced()).foregroundStyle(.secondary)
                    Text("proposer \(b.proposer.prefix(10))…").font(.caption2.monospaced()).foregroundStyle(.secondary)
                }
            }
        } label: { Text("Finalized blocks") }
    }
}
