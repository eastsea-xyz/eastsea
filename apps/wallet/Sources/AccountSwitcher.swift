import SwiftUI

@MainActor
struct AccountSwitcherButton: View {
    @ObservedObject var store: AccountStore
    var compact = false
    @State private var open = false

    var body: some View {
        Button { open.toggle() } label: {
            HStack(spacing: DesignTokens.Space.s2) {
                AccountIcon(address: store.activeAccount?.address, size: compact ? 16 : 20)
                VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                    Text(verbatim: store.activeAccount?.name ?? String(localized: "Accounts"))
                        .font(.aeFootnote.weight(.semibold)).lineLimit(1)
                    if let account = store.activeAccount {
                        Text(verbatim: Short.address(account.address))
                            .font(.aeCaption.monospaced()).foregroundStyle(DesignTokens.Palette.textMuted.color).lineLimit(1)
                    }
                }
                Image(systemName: "chevron.down").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
            }
            .padding(.horizontal, compact ? DesignTokens.Space.s2 : DesignTokens.Space.s3)
            .padding(.vertical, DesignTokens.Space.s2)
            .foregroundStyle(DesignTokens.Palette.text.color)
            .background(DesignTokens.Palette.surfaceSunken.color,
                        in: RoundedRectangle(cornerRadius: compact ? DesignTokens.Radius.sm : DesignTokens.Radius.lg))
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
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            HStack(spacing: DesignTokens.Space.s2) {
                EastSeaDawnMark().frame(width: 24, height: 24)
                Text("Accounts").font(.aeTitle)
                Spacer()
                Button("Done") { dismiss() }.buttonStyle(.plain)
                    .foregroundStyle(DesignTokens.Palette.textMuted.color)
            }
            ScrollView {
                VStack(spacing: DesignTokens.Space.s1) {
                    ForEach(store.list()) { account in row(account) }
                }
            }
            .scrollIndicators(.hidden)
            .frame(height: min(CGFloat(max(1, store.list().count)) * 64, 300))
            if let error {
                Text(error).font(.aeFootnote).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Rectangle().fill(DesignTokens.Palette.line.color).frame(height: 1)
            Button { creating = true } label: { Label("Create account", systemImage: "plus") }
                .buttonStyle(EastSeaPrimaryButtonStyle()).disabled(store.state != .ready)
            Text("A small one-time fee on first use").font(.aeCaption).foregroundStyle(DesignTokens.Palette.textMuted.color)
                .fixedSize(horizontal: false, vertical: true)
            if !store.retiredAccountIDs.isEmpty {
                Menu("Retired accounts") {
                    ForEach(store.retiredAccountIDs, id: \.self) { id in
                        Button("Restore Account \(id)") { perform { try store.restoreDeleted(id) } }
                    }
                }
            }
        }
        .padding(DesignTokens.Space.s4).frame(width: 336)
        .eastSeaSheet()
        .sheet(isPresented: $creating) { AccountNameSheet(store: store) }
        .sheet(item: $renaming) { AccountNameSheet(store: store, account: $0) }
        .sheet(item: $retiring) { RetireAccountView(store: store, account: $0) }
    }

    private func row(_ account: WalletAccount) -> some View {
        HStack(spacing: DesignTokens.Space.s2) {
            Button {
                perform { try store.select(account.id); dismiss() }
            } label: {
                HStack(spacing: DesignTokens.Space.s3) {
                    AccountIcon(address: account.address, size: 28)
                    VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                        Text(verbatim: account.name).font(.aeBody.weight(.medium)).lineLimit(1)
                        Text(verbatim: Short.address(account.address)).font(.aeCaption.monospaced())
                            .foregroundStyle(DesignTokens.Palette.textMuted.color).lineLimit(1)
                    }
                    Spacer(minLength: 4)
                    if store.activeAccount?.id == account.id {
                        Image(systemName: "checkmark").foregroundStyle(Color.aether)
                            .accessibilityLabel("Selected account")
                    }
                }
                .padding(DesignTokens.Space.s3).contentShape(Rectangle())
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
        .background(store.activeAccount?.id == account.id ? DesignTokens.Palette.surfaceSunken.color : .clear,
                    in: RoundedRectangle(cornerRadius: DesignTokens.Radius.md))
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
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            HStack(spacing: DesignTokens.Space.s2) {
                EastSeaDawnMark().frame(width: 24, height: 24)
                Text(account == nil ? String(localized: "Create account") : String(localized: "Rename account"))
                    .font(.aeTitle)
            }
            TextField("Account name", text: $name).textFieldStyle(EastSeaTextFieldStyle()).font(.aeBody)
            if account == nil {
                Text("A small one-time fee on first use").font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let error { Text(error).font(.aeFootnote).foregroundStyle(Color.warn) }
            HStack {
                Button("Cancel") { dismiss() }.buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Spacer()
                Button(account == nil ? String(localized: "Create account") : String(localized: "Save")) {
                    do {
                        if let account { try store.rename(account.id, name: name) }
                        else { try store.createAndSelect(name: name) }
                        dismiss()
                    } catch { self.error = error.localizedDescription }
                }
                .buttonStyle(EastSeaPrimaryButtonStyle()).keyboardShortcut(.defaultAction)
                .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(DesignTokens.Space.s6).frame(width: 340)
        .eastSeaSheet()
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
        VStack(alignment: .leading, spacing: DesignTokens.Space.s4) {
            Text("Retire account").font(.aeTitle)
            VStack(alignment: .leading, spacing: DesignTokens.Space.s1) {
                Text(verbatim: account.name).font(.aeBody.weight(.semibold))
                Text(verbatim: Short.address(account.address)).font(.aeFootnote.monospaced())
                    .foregroundStyle(DesignTokens.Palette.textMuted.color)
            }
            .padding(DesignTokens.Space.s4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(DesignTokens.Palette.surfaceSunken.color, in: RoundedRectangle(cornerRadius: DesignTokens.Radius.md))
            if let failure {
                Label(reason(failure), systemImage: "info.circle").font(.aeBody).foregroundStyle(Color.warn)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Check balance again") { self.failure = store.retirementFailure(for: account.id); error = nil }
                    .buttonStyle(EastSeaQuietButtonStyle())
            } else {
                Text("The key stays on this Mac. You can restore this account later.")
                    .font(.aeFootnote).foregroundStyle(DesignTokens.Palette.textMuted.color).fixedSize(horizontal: false, vertical: true)
            }
            if let error { Text(error).font(.aeFootnote).foregroundStyle(Color.warn) }
            HStack {
                Button("Cancel") { dismiss() }.buttonStyle(EastSeaQuietButtonStyle()).keyboardShortcut(.cancelAction)
                Spacer()
                Button("Retire account", role: .destructive) {
                    do { try store.delete(account.id); dismiss() }
                    catch {
                        self.failure = store.retirementFailure(for: account.id)
                        self.error = error.localizedDescription
                    }
                }
                .buttonStyle(EastSeaPrimaryButtonStyle())
                .disabled(failure != nil)
            }
        }
        .padding(DesignTokens.Space.s6).frame(width: 360)
        .eastSeaSheet()
    }

    private func reason(_ failure: AccountStore.Failure) -> String {
        if case .balanceNotZero = failure {
            return String(localized: "This account still has funds. Move its balance and tokens before retiring it so you can keep using them.")
        }
        return failure.localizedDescription
    }
}
