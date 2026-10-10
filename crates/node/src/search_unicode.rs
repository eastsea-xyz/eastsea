//! UTS-39 skeletons pinned to Unicode 16.0.0: NFD, confusable mapping, NFD.
//! Tables are shipped locally so every node computes the same warning.
//! This is a comparison key, never a replacement for the displayed name.

#[path = "search-unicode/tables.rs"]
mod tables;

fn mapping(rows: &'static [(u32, &'static str)], ch: char) -> Option<&'static str> {
    rows.binary_search_by_key(&(ch as u32), |row| row.0)
        .ok()
        .map(|i| rows[i].1)
}

fn combining_class(ch: char) -> u8 {
    tables::COMBINING_CLASSES
        .binary_search_by_key(&(ch as u32), |row| row.0)
        .ok()
        .map(|i| tables::COMBINING_CLASSES[i].1)
        .unwrap_or(0)
}

fn decompose(ch: char, out: &mut Vec<char>) {
    let point = ch as u32;
    // Hangul syllable decomposition (UAX-15 §3.12).
    if (0xac00..0xd7a4).contains(&point) {
        let syllable = point - 0xac00;
        out.push(char::from_u32(0x1100 + syllable / 588).expect("Hangul L"));
        out.push(char::from_u32(0x1161 + (syllable % 588) / 28).expect("Hangul V"));
        if !syllable.is_multiple_of(28) {
            out.push(char::from_u32(0x11a7 + syllable % 28).expect("Hangul T"));
        }
    } else if let Some(parts) = mapping(tables::DECOMPOSITIONS, ch) {
        for part in parts.chars() {
            decompose(part, out);
        }
    } else {
        out.push(ch);
    }
}

fn nfd(text: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        decompose(ch, &mut out);
    }
    // Stable canonical ordering within each combining sequence, including
    // an initial sequence with no starter.
    let mut start = 0;
    for i in 0..out.len() {
        if combining_class(out[i]) == 0 {
            out[start..i].sort_by_key(|ch| combining_class(*ch));
            start = i + 1;
        }
    }
    out[start..].sort_by_key(|ch| combining_class(*ch));
    out
}

pub fn skeleton(text: &str) -> String {
    let mut mapped = String::with_capacity(text.len());
    for ch in nfd(text) {
        if let Some(replacement) = mapping(tables::CONFUSABLES, ch) {
            mapped.push_str(replacement);
        } else {
            mapped.push(ch);
        }
    }
    nfd(&mapped).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uts39_ascii_and_cross_script_confusables() {
        for (a, b) in [
            ("modern.sea", "rnodern.sea"),
            ("lily.sea", "1i1y.sea"),
            ("paypal", "pаypal"),
            ("O", "0"),
            ("é", "e\u{301}"),
            ("한", "\u{1112}\u{1161}\u{11ab}"),
        ] {
            assert_eq!(skeleton(a), skeleton(b), "{a} / {b}");
        }
        assert_ne!(skeleton("eastsea"), skeleton("other"));
    }

    #[test]
    fn canonical_order_is_stable_and_handles_leading_marks() {
        assert_eq!(nfd("a\u{315}\u{300}"), nfd("a\u{300}\u{315}"));
        assert_eq!(nfd("\u{315}\u{300}a"), nfd("\u{300}\u{315}a"));
        assert_eq!(nfd("é"), vec!['e', '\u{301}']);
    }
}
