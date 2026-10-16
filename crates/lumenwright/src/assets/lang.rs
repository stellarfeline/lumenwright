//! Translation tables (`assets/<ns>/lang/<code>.json`) from the jar and resource packs,
//! for text components that name a `translate` key.

use std::collections::HashMap;

use super::source::ResourceStore;

/// The language Lumenwright resolves `translate` keys in (the client's default).
pub const DEFAULT_LANGUAGE: &str = "en_us";

/// One language's translation table: `translate` keys to their text.
#[derive(Clone, Debug, Default)]
pub struct Lang {
    entries: HashMap<String, String>,
}

impl Lang {
    /// Merges every namespace's `lang/<code>.json`; a later resource layer already
    /// replaced an earlier one's file of the same path in the store.
    pub fn load(store: &ResourceStore, code: &str) -> Lang {
        let mut lang = Lang::default();
        let suffix = format!("/lang/{code}.json");
        for path in store.paths_ending_with(&suffix) {
            let Some(bytes) = store.get(&path) else {
                continue;
            };
            match serde_json::from_slice::<HashMap<String, String>>(bytes) {
                Ok(map) => lang.entries.extend(map),
                Err(e) => log::warn!("{path}: {e}"),
            }
        }
        lang
    }

    /// The text for `key`, if the table has it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|s| s.as_str())
    }

    /// Adds or replaces the text for `key`.
    pub fn insert(&mut self, key: &str, value: &str) {
        self.entries.insert(key.to_string(), value.to_string());
    }

    /// `key` formatted with `args` (`%s` in order, `%1$s` by position, `%%`), as
    /// vanilla's `TranslatableContents` does; `fallback`, then the key itself, when
    /// the key is unknown.
    pub fn format(&self, key: &str, fallback: Option<&str>, args: &[String]) -> String {
        let Some(pattern) = self.get(key).or(fallback) else {
            return key.to_string();
        };
        let mut out = String::new();
        let mut next = 0;
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.peek() {
                Some('%') => {
                    chars.next();
                    out.push('%');
                }
                Some('s') => {
                    chars.next();
                    out.push_str(args.get(next).map(|s| s.as_str()).unwrap_or(""));
                    next += 1;
                }
                Some(d) if d.is_ascii_digit() => {
                    let mut n = String::new();
                    while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                        n.push(*d);
                        chars.next();
                    }
                    if chars.next_if_eq(&'$').is_some() && chars.next_if_eq(&'s').is_some() {
                        let i = n.parse::<usize>().unwrap_or(1).saturating_sub(1);
                        out.push_str(args.get(i).map(|s| s.as_str()).unwrap_or(""));
                    } else {
                        out.push('%');
                        out.push_str(&n);
                    }
                }
                _ => out.push('%'),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_translatable_contents() {
        let mut store = ResourceStore::new();
        store.insert(
            "assets/minecraft/lang/en_us.json",
            br#"{"a": "%s and %s", "b": "%2$s before %1$s, 100%%"}"#.to_vec(),
        );
        store.insert(
            "assets/delve/lang/en_us.json",
            br#"{"slate": "FERRY"}"#.to_vec(),
        );
        let l = Lang::load(&store, "en_us");
        let args = ["x".to_string(), "y".to_string()];
        assert_eq!(l.format("a", None, &args), "x and y");
        assert_eq!(l.format("b", None, &args), "y before x, 100%");
        assert_eq!(l.format("slate", None, &[]), "FERRY");
        assert_eq!(l.format("nope", Some("fb"), &[]), "fb");
        assert_eq!(l.format("nope", None, &[]), "nope");
    }
}
