//! A minimal reader for Valve's KeyValues text format.
//!
//! Steam stores both `libraryfolders.vdf` and `appmanifest_*.acf` in this
//! format. Only what is needed to locate an install is supported: quoted
//! keys/values, nested blocks, and `//` comments.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Block(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Block(map) => map
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v),
            Value::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            Value::Block(_) => None,
        }
    }

    /// Convenience for `node.get(key)?.as_text()`.
    pub fn text_at(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_text()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&String, &Value)> {
        match self {
            Value::Block(map) => map.iter(),
            Value::Text(_) => EMPTY.iter(),
        }
    }
}

static EMPTY: BTreeMap<String, Value> = BTreeMap::new();

/// Parse a KeyValues document into a single root block.
pub fn parse(text: &str) -> Value {
    let tokens = tokenize(text);
    let mut cursor = 0;
    let mut root = BTreeMap::new();
    parse_pairs(&tokens, &mut cursor, &mut root);
    Value::Block(root)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Str(String),
    Open,
    Close,
}

fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '/' if i + 1 < chars.len() && chars[i + 1] == '/' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '{' => {
                tokens.push(Token::Open);
                i += 1;
            }
            '}' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '"' => {
                i += 1;
                let mut buf = String::new();
                while i < chars.len() && chars[i] != '"' {
                    // Steam escapes backslashes in Windows paths.
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        buf.push(chars[i]);
                    } else {
                        buf.push(chars[i]);
                    }
                    i += 1;
                }
                i += 1; // closing quote
                tokens.push(Token::Str(buf));
            }
            _ => {
                // Unquoted token, tolerated for robustness.
                let mut buf = String::new();
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != '{'
                    && chars[i] != '}'
                {
                    buf.push(chars[i]);
                    i += 1;
                }
                tokens.push(Token::Str(buf));
            }
        }
    }
    tokens
}

fn parse_pairs(tokens: &[Token], cursor: &mut usize, out: &mut BTreeMap<String, Value>) {
    while *cursor < tokens.len() {
        match &tokens[*cursor] {
            Token::Close => {
                *cursor += 1;
                return;
            }
            Token::Open => {
                // A block with no key; skip it rather than fail the whole file.
                *cursor += 1;
                let mut discard = BTreeMap::new();
                parse_pairs(tokens, cursor, &mut discard);
            }
            Token::Str(key) => {
                let key = key.clone();
                *cursor += 1;
                match tokens.get(*cursor) {
                    Some(Token::Open) => {
                        *cursor += 1;
                        let mut block = BTreeMap::new();
                        parse_pairs(tokens, cursor, &mut block);
                        out.insert(key, Value::Block(block));
                    }
                    Some(Token::Str(value)) => {
                        out.insert(key, Value::Text(value.clone()));
                        *cursor += 1;
                    }
                    _ => return,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY_FOLDERS: &str = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\\Program Files (x86)\\Steam"
        "apps"
        {
            "3489700"		"98765432"
        }
    }
    "1"
    {
        "path"		"D:\\SteamLibrary"
        "apps"
        {
        }
    }
}
"#;

    #[test]
    fn reads_nested_blocks_and_unescapes_paths() {
        let root = parse(LIBRARY_FOLDERS);
        let folders = root.get("libraryfolders").unwrap();
        assert_eq!(
            folders.get("0").unwrap().text_at("path"),
            Some(r"C:\Program Files (x86)\Steam")
        );
        assert_eq!(
            folders.get("1").unwrap().text_at("path"),
            Some(r"D:\SteamLibrary")
        );
    }

    #[test]
    fn finds_an_app_inside_a_library() {
        let root = parse(LIBRARY_FOLDERS);
        let apps = root
            .get("libraryfolders")
            .unwrap()
            .get("0")
            .unwrap()
            .get("apps")
            .unwrap();
        assert!(apps.get("3489700").is_some());
    }

    #[test]
    fn reads_an_app_manifest() {
        let manifest = r#"
"AppState"
{
    "appid"		"3489700"
    "name"		"Stellar Blade"
    "installdir"		"StellarBlade"
}
"#;
        let root = parse(manifest);
        assert_eq!(
            root.get("AppState").unwrap().text_at("installdir"),
            Some("StellarBlade")
        );
    }

    #[test]
    fn ignores_comments() {
        let root = parse("// leading note\n\"a\"\t\"b\"\n");
        assert_eq!(root.text_at("a"), Some("b"));
    }
}
