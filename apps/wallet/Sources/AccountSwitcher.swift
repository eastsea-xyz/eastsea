import SwiftUI

@MainActor
struct AccountSwitcherButton: View {
    @ObservedObject var store: AccountStore
    var compact = false
    @State private var open = false

    var body: some View {
        Button { open.toggle() } label: {
            HStack(spacing: 6) {
                AccountDot(account: store.activeAccount, size: compact ? 14 : 20)
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: store.activeAccount?.name ?? String(localized: "Accounts"))
                        .font(.aeFootnote.weight(.semibold)).lineLimit(1)
                    if let account = store.activeAccount {
                        Text(verbatim: Short.address(account.address))
                            .font(.aeCaption.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                    }
                }
                Image(systemName: "chevron.down").font(.caption2).foregroundStyle(.secondary)
            }
            .padding(.horizontal, compact ? 6 : 12).padding(.vertical, 6)
            .background(.background.secondary, in: RoundedRectangle(cornerRadius: compact ? 8 : 16))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Switch account")
        .accessibilityLabel("Switch account")
        .accessibilityValue(store.activeAccount?.name ?? "")
        .popover(isPresented: $open) { AccountSwitcherPanel(store: store) }
    }
}

@MainActor
struct AccountSwitcherPanel: View {
    @ObservedObject var store: AccountStore
    @Environment(\.dismiss) private var dismiss
    @State private var creating = false
    @State private var renaming: WalletAccount?
    @State private var retiring: WalletAccount?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("Accounts").font(.aeHeadline)
                Spacer()
                Button("Done") { dismiss() }.buttonStyle(.plain).foregroundStyle(.secondary)
            }
            ScrollView {
                VStack(spacing: 4) {
                    ForEach(store.list()) { account in row(account) }
                }
            }
            .scrollIndicators(.hidden)
            .frame(height: min(CGFloat(max(1, store.list().count)) * 64, 300))
            if let error {
                Text(error).font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Divider()
            Button { creating = true } label: { Label("Create account", systemImage: "plus") }
                .buttonStyle(.bordered).disabled(store.state != .ready)
            Text("A small one-time fee on first use").font(.aeCaption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if !store.retiredAccountIDs.isEmpty {
                Menu("Retired accounts") {
                    ForEach(store.retiredAccountIDs, id: \.self) { id in
                        Button("Restore Account \(id)") { perform { try store.restoreDeleted(id) } }
                    }
                }
            }
        }
        .padding(18).frame(width: 340)
        .sheet(isPresented: $creating) { AccountNameSheet(store: store) }
        .sheet(item: $renaming) { AccountNameSheet(store: store, account: $0) }
        .sheet(item: $retiring) { RetireAccountView(store: store, account: $0) }
    }

    private func row(_ account: WalletAccount) -> some View {
        HStack(spacing: 8) {
            Button {
                perform { try store.select(account.id); dismiss() }
            } label: {
                HStack(spacing: 10) {
                    AccountDot(account: account, size: 28)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(verbatim: account.name).font(.aeBody.weight(.medium)).lineLimit(1)
                        Text(verbatim: Short.address(account.address)).font(.aeCaption.monospaced())
                            .foregroundStyle(.secondary).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    if store.activeAccount?.id == account.id {
                        Image(systemName: "checkmark").foregroundStyle(Color.aether)
                            .accessibilityLabel("Selected account")
                    }
                }
                .padding(10).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            Menu {
                Button("Rename…") { renaming = account }
                Button("Move up") { move(account, by: -1) }.disabled(store.list().first?.id == account.id)
                Button("Move down") { move(account, by: 1) }.disabled(store.list().last?.id == account.id)
                Divider()
                Button("Retire account…", role: .destructive) { retiring = account }
            } label: { Image(systemName: "ellipsis").padding(8) }
            #if os(macOS)
            .menuStyle(.borderlessButton)
            #endif
            .fixedSize().help("Account actions")
        }
        .background(store.activeAccount?.id == account.id ? Color.aether.opacity(0.08) : .clear,
                    in: RoundedRectangle(cornerRadius: 10))
    }

    private func move(_ account: WalletAccount, by offset: Int) {
        var ids = store.list().map(\.id)
        guard let index = ids.firstIndex(of: account.id), ids.indices.contains(index + offset) else { return }
        ids.swapAt(index, index + offset)
        perform { try store.reorder(ids) }
    }

    private func perform(_ action: () throws -> Void) {
        do { try action(); error = nil }
        catch { self.error = error.localizedDescription }
    }
}

@MainActor
private struct AccountNameSheet: View {
    @ObservedObject var store: AccountStore
    let account: WalletAccount?
    @Environment(\.dismiss) private var dismiss
    @State private var name: String
    @State private var error: String?

    init(store: AccountStore, account: WalletAccount? = nil) {
        self.store = store
        self.account = account
        let nextID = (store.accounts.map(\.id) + store.retiredAccountIDs).max().map { $0 + 1 } ?? 1
        _name = State(initialValue: account?.name ?? String(localized: "Account \(nextID)"))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(account == nil ? String(localized: "Create account") : String(localized: "Rename account"))
                .font(.aeHeadline)
            TextField("Account name", text: $name).textFieldStyle(.roundedBorder)
            if account == nil {
                Text("A small one-time fee on first use").font(.aeFootnote).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error { Text(error).font(.aeFootnote).foregroundStyle(Color.warn) }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button(account == nil ? String(localized: "Create account") : String(localized: "Save")) {
                    do {
                        if let account { try store.rename(account.id, name: name) }
                        else { try store.createAndSelect(name: name) }
                        dismiss()
                    } catch { self.error = error.localizedDescription }
                }
                .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(24).frame(width: 340)
    }
}

@MainActor
struct RetireAccountView: View {
    @ObservedObject var store: AccountStore
    let account: WalletAccount
    @Environment(\.dismiss) private var dismiss
    @State private var failure: AccountStore.Failure?
    @State private var error: String?

    init(store: AccountStore, account: WalletAccount) {
        self.store = store
        self.account = account
        _failure = State(initialValue: store.retirementFailure(for: account.id))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Retire account").font(.aeHeadline)
            Text(verbatim: account.name).font(.aeBody.weight(.semibold))
            Text(verbatim: Short.address(account.address)).font(.aeFootnote.monospaced()).foregroundStyle(.secondary)
            if let failure {
                Label(reason(failure), systemImage: "info.circle").font(.aeBody)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Check balance again") { self.failure = store.retirementFailure(for: account.id); error = nil }
            } else {
                Text("The key stays on this Mac. You can restore this account later.")
                    .font(.aeFootnote).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            if let error { Text(error).font(.aeFootnote).foregroundStyle(Color.warn) }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Retire account", role: .destructive) {
                    do { try store.delete(account.id); dismiss() }
                    catch {
                        self.failure = store.retirementFailure(for: account.id)
                        self.error = error.localizedDescription
                    }
                }
                .disabled(failure != nil)
            }
        }
        .padding(24).frame(width: 360)
    }

    private func reason(_ failure: AccountStore.Failure) -> String {
        if case .balanceNotZero = failure {
            return String(localized: "This account still has funds. Move its balance and tokens before retiring it so you can keep using them.")
        }
        return failure.localizedDescription
    }
}

private struct AccountDot: View {
    let account: WalletAccount?
    let size: CGFloat
    private var color: Color {
        switch account?.color {
        case "blue": .blue
        case "green": .green
        case "orange": .orange
        case "pink": .pink
        default: .aether
        }
    }
    var body: some View { Circle().fill(color).frame(width: size, height: size).accessibilityHidden(true) }
}
