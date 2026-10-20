//! The wildcard matching `FindFirstFileW` does.
//!
//! When the overlay answers a directory search itself, it has to filter the
//! merged listing the way Windows would have, or the engine sees files its
//! pattern excluded — or misses paks because `*.pak` was matched as text.

/// Whether `name` matches a Win32 search `pattern`, ignoring case.
///
/// `*` is any run of characters and `?` any one. `*.*` matches every name,
/// including names with no dot, because that is what Windows does for the
/// pattern every old program uses to mean "all files".
pub fn matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "*.*" || pattern.is_empty() {
        return true;
    }
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    glob(&pattern, &name)
}

fn glob(pattern: &[char], name: &[char]) -> bool {
    // Iterative with one backtrack point, so a pathological pattern cannot
    // take exponential time inside a file system hook.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn everything_matches_the_catch_all_patterns() {
        for pattern in ["*", "*.*", ""] {
            assert!(matches(pattern, "pakchunk0.pak"));
            assert!(matches(pattern, "NoExtension"));
        }
    }

    #[test]
    fn extensions_match_without_regard_to_case() {
        assert!(matches("*.pak", "0010_Outfit_P.PAK"));
        assert!(!matches("*.pak", "0010_Outfit_P.utoc"));
        assert!(!matches("*.pak", "x.pak.bak"));
        assert!(matches("*_P.*", "Outfit_p.ucas"));
    }

    #[test]
    fn question_marks_take_exactly_one_character() {
        assert!(matches("pakchunk?.pak", "pakchunk0.pak"));
        assert!(!matches("pakchunk?.pak", "pakchunk10.pak"));
    }

    #[test]
    fn an_exact_name_matches_only_itself() {
        assert!(matches("mods.txt", "Mods.TXT"));
        assert!(!matches("mods.txt", "mods.txt.bak"));
    }
}
