import Foundation

/// What recovery cannot do (contracts audit 2026-10-05, F-05): with EIP-7702
/// delegation the account's original signer keeps its authority after a
/// guardian recovery or an added owner key. Recovery saves a LOST key; it
/// never revokes a STOLEN one, so the wallet says so wherever recovery or the
/// key's custody is explained, and tells people to move their funds instead.
/// Pure Foundation, so scripts/test-swift-pure.sh can check the wording.
enum KeyExposureNotice {
    /// Next to recovery (devices, words, "recover a lost account").
    case recovery
    /// Next to where the key lives and what protects it.
    case keyCustody

    func text(korean ko: Bool) -> String {
        switch self {
        case .recovery:
            return ko
                ? "복구는 잃어버린 키 대신 자금을 옮겨 줄 뿐, 이 계정의 원래 키를 막지는 못해요. 원래 키가 노출됐을 수 있다면(이 Mac이 잠금 해제된 채 남의 손에 있었거나 백업이 새어 나갔다면) 잔액 전부를 새 계정으로 옮기세요."
                : "Recovery moves funds when a key is lost; it does not lock this account's original key out. If that key may be exposed (someone had this Mac unlocked, or a backup leaked), move all your funds to a new account."
        case .keyCustody:
            return ko
                ? "원래 키가 노출됐을 수 있다면 복구 기기나 복구 단어로는 그 키를 막을 수 없어요. 바로 잔액 전부를 새 계정으로 옮기세요."
                : "If this account's original key may be exposed, recovery devices and words cannot lock it out. Move all your funds to a new account right away."
        }
    }

    /// In the language the app itself is shown in (its bundle localization), so a
    /// Korean-preferring Mac never sees one Korean line in an English screen.
    var text: String { text(korean: Bundle.main.preferredLocalizations.first?.hasPrefix("ko") ?? false) }
}
