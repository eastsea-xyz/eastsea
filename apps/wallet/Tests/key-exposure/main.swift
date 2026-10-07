// F-05 (contracts audit 2026-10-05): recovery never revokes a stolen original
// key, so every notice must say to move the funds and must never promise that
// recovery locks the old key out. Pure Foundation: no FFI, no SwiftUI.
import Foundation

func check(_ ok: Bool, _ name: String) {
    if ok { print("ok   \(name)") } else { print("FAIL \(name)"); exit(1) }
}

for notice in [KeyExposureNotice.recovery, .keyCustody] {
    let en = notice.text(locale: walletTestLocale("en"), bundle: walletTestBundle("en")), ko = notice.text(locale: walletTestLocale("ko"), bundle: walletTestBundle("ko"))
    check(en.contains("move all your funds to a new account") || en.contains("Move all your funds to a new account"), "\(notice) en says to move funds to a new account")
    check(en.contains("not lock") || en.contains("cannot lock"), "\(notice) en says recovery does not lock the key out")
    check(ko.contains("새 계정으로 옮기세요"), "\(notice) ko says to move funds to a new account")
    check(ko.contains("막지는 못해요") || ko.contains("막을 수 없어요"), "\(notice) ko says recovery does not lock the key out")
    check(!ko.replacingOccurrences(of: "Mac", with: "").contains(where: { $0.isASCII && $0.isLetter }), "\(notice) ko has no untranslated English")
    // Never the false comfort the audit warns about.
    check(!en.localizedCaseInsensitiveContains("recovery revokes") && !en.localizedCaseInsensitiveContains("recovery locks"), "\(notice) en never claims recovery revokes the key")
    let ja = notice.text(locale: walletTestLocale("ja"), bundle: walletTestBundle("ja"))
    check(ja.contains("新しいアカウント") && ja.contains("キー"), "\(notice) Japanese keeps the key-exposure warning")
    check(!ja.unicodeScalars.contains { (0xAC00...0xD7A3).contains($0.value) }, "\(notice) Japanese has no Korean")
}
