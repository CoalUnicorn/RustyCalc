/// Spreadsheet column label for a 1-based column index ("A", "Z", "AA",...).
/// Bijective base-26; returns "" for `col < 1`.
pub fn col_name(col: i32) -> String {
    if col < 1 {
        return String::new();
    }
    let mut n = col;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    // Safety: out contains only ASCII uppercase letters (b'A'..=b'Z').
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::col_name;

    #[test]
    fn col_name_is_bijective_base26() {
        assert_eq!(col_name(1), "A");
        assert_eq!(col_name(26), "Z");
        assert_eq!(col_name(27), "AA");
        assert_eq!(col_name(52), "AZ");
        assert_eq!(col_name(53), "BA");
        assert_eq!(col_name(703), "AAA");
    }

    #[test]
    fn col_name_below_one_is_empty() {
        assert_eq!(col_name(0), "");
        assert_eq!(col_name(-5), "");
    }
}
