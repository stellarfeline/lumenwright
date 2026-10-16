//! Text components as stored in a save (sign messages), flattened to styled runs and
//! laid out with the vanilla bitmap font.

use fastnbt::Value;

use crate::assets::font::Font;
use crate::assets::lang::Lang;
use crate::world::nbt;

/// A run of characters with one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// The characters of the run.
    pub text: String,
    /// RGB colour set by the component's style; `None` takes the caller's colour.
    pub color: Option<u32>,
    /// Drawn in the bold face (each glyph struck twice, one pixel apart).
    pub bold: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Style {
    color: Option<u32>,
    bold: bool,
}

/// `ChatFormatting` colours by name.
pub fn named_color(name: &str) -> Option<u32> {
    Some(match name {
        "black" => 0x000000,
        "dark_blue" => 0x0000AA,
        "dark_green" => 0x00AA00,
        "dark_aqua" => 0x00AAAA,
        "dark_red" => 0xAA0000,
        "dark_purple" => 0xAA00AA,
        "gold" => 0xFFAA00,
        "gray" => 0xAAAAAA,
        "dark_gray" => 0x555555,
        "blue" => 0x5555FF,
        "green" => 0x55FF55,
        "aqua" => 0x55FFFF,
        "red" => 0xFF5555,
        "light_purple" => 0xFF55FF,
        "yellow" => 0xFFFF55,
        "white" => 0xFFFFFF,
        _ => {
            let hex = name.strip_prefix('#')?;
            return u32::from_str_radix(hex, 16).ok().filter(|_| hex.len() == 6);
        }
    })
}

/// Flattens a component stored as NBT (1.21.5+: a string, a compound or a list) or
/// as a JSON string (older saves).
pub fn flatten(v: &Value, lang: &Lang) -> Vec<Span> {
    let mut out = Vec::new();
    match v {
        Value::String(s) => {
            let t = s.trim_start();
            if (t.starts_with('{') || t.starts_with('[') || t.starts_with('"'))
                && let Ok(j) = serde_json::from_str::<serde_json::Value>(s)
            {
                flatten_json(&j, Style::default(), lang, &mut out);
            } else {
                push(&mut out, s, Style::default());
            }
        }
        _ => flatten_nbt(v, Style::default(), lang, &mut out),
    }
    out
}

/// The plain text of a component.
pub fn plain(v: &Value, lang: &Lang) -> String {
    flatten(v, lang).into_iter().map(|s| s.text).collect()
}

fn push(out: &mut Vec<Span>, text: &str, st: Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.color == st.color
        && last.bold == st.bold
    {
        last.text.push_str(text);
        return;
    }
    out.push(Span {
        text: text.to_string(),
        color: st.color,
        bold: st.bold,
    });
}

fn flatten_nbt(v: &Value, parent: Style, lang: &Lang, out: &mut Vec<Span>) {
    match v {
        Value::String(s) => push(out, s, parent),
        Value::List(l) => {
            // A list is its first element with the rest as siblings inheriting its style.
            let mut it = l.iter();
            if let Some(first) = it.next() {
                flatten_nbt(first, parent, lang, out);
                let st = nbt::compound(first)
                    .map(|c| nbt_style(c, parent))
                    .unwrap_or(parent);
                for v in it {
                    flatten_nbt(v, st, lang, out);
                }
            }
        }
        Value::Compound(c) => {
            let st = nbt_style(c, parent);
            if let Some(t) = nbt::get_str(c, "text") {
                push(out, t, st);
            } else if let Some(k) = nbt::get_str(c, "translate") {
                let args: Vec<String> = match c.get("with") {
                    Some(Value::List(l)) => l.iter().map(|a| plain(a, lang)).collect(),
                    _ => Vec::new(),
                };
                push(out, &lang.format(k, nbt::get_str(c, "fallback"), &args), st);
            } else if let Some(k) = nbt::get_str(c, "keybind") {
                push(out, k, st);
            } else if let Some(v) = c.get("") {
                flatten_nbt(v, st, lang, out);
            }
            if let Some(Value::List(extra)) = c.get("extra") {
                for e in extra {
                    flatten_nbt(e, st, lang, out);
                }
            }
        }
        // Numbers and booleans stringify (`{text: 1}` is rare but legal SNBT).
        other => {
            if let Some(n) = nbt::as_f64(other) {
                push(out, &n.to_string(), parent);
            }
        }
    }
}

fn nbt_style(c: &nbt::Compound, parent: Style) -> Style {
    Style {
        color: nbt::get_str(c, "color")
            .and_then(named_color)
            .or(parent.color),
        bold: match c.get("bold") {
            Some(v) => nbt::as_i64(v).unwrap_or(0) != 0,
            None => parent.bold,
        },
    }
}

fn flatten_json(v: &serde_json::Value, parent: Style, lang: &Lang, out: &mut Vec<Span>) {
    use serde_json::Value as J;
    match v {
        J::String(s) => push(out, s, parent),
        J::Number(n) => push(out, &n.to_string(), parent),
        J::Bool(b) => push(out, &b.to_string(), parent),
        J::Array(l) => {
            let mut it = l.iter();
            if let Some(first) = it.next() {
                flatten_json(first, parent, lang, out);
                let st = first
                    .as_object()
                    .map(|o| json_style(o, parent))
                    .unwrap_or(parent);
                for v in it {
                    flatten_json(v, st, lang, out);
                }
            }
        }
        J::Object(o) => {
            let st = json_style(o, parent);
            if let Some(t) = o.get("text") {
                flatten_json(t, st, lang, out);
            } else if let Some(k) = o.get("translate").and_then(|k| k.as_str()) {
                let args: Vec<String> = match o.get("with") {
                    Some(J::Array(l)) => l
                        .iter()
                        .map(|a| {
                            let mut v = Vec::new();
                            flatten_json(a, Style::default(), lang, &mut v);
                            v.into_iter().map(|s| s.text).collect()
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let f = o.get("fallback").and_then(|f| f.as_str());
                push(out, &lang.format(k, f, &args), st);
            } else if let Some(k) = o.get("keybind").and_then(|k| k.as_str()) {
                push(out, k, st);
            }
            if let Some(J::Array(extra)) = o.get("extra") {
                for e in extra {
                    flatten_json(e, st, lang, out);
                }
            }
        }
        J::Null => {}
    }
}

fn json_style(o: &serde_json::Map<String, serde_json::Value>, parent: Style) -> Style {
    Style {
        color: o
            .get("color")
            .and_then(|c| c.as_str())
            .and_then(named_color)
            .or(parent.color),
        bold: o
            .get("bold")
            .and_then(|b| b.as_bool())
            .unwrap_or(parent.bold),
    }
}

/// A character placed on a line, in font pixels from the line's left edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// The character.
    pub c: char,
    /// Left edge of the glyph, in font pixels from the line's left edge.
    pub x: f32,
    /// RGB colour set by the style; `None` takes the caller's colour.
    pub color: Option<u32>,
    /// Drawn in the bold face.
    pub bold: bool,
}

/// Lays out one line and returns its characters and width. Like the sign renderer
/// (`Font.split(line, maxWidth)`, first line kept), text past `max_width` is cut at
/// the last space that fits, or mid-word when no space does.
pub fn layout_line(font: &Font, spans: &[Span], max_width: f32) -> (Vec<Placed>, f32) {
    let chars: Vec<(char, Option<u32>, bool)> = spans
        .iter()
        .flat_map(|s| s.text.chars().map(move |c| (c, s.color, s.bold)))
        .take_while(|(c, _, _)| *c != '\n')
        .collect();
    let mut width = 0.0;
    let mut end = chars.len();
    let mut last_space: Option<usize> = None;
    for (i, &(c, _, bold)) in chars.iter().enumerate() {
        let a = font.advance(c, bold);
        if width + a > max_width {
            end = match last_space {
                Some(s) => s,
                None => i,
            };
            break;
        }
        if c == ' ' {
            last_space = Some(i);
        }
        width += a;
    }
    let mut x = 0.0;
    let mut placed = Vec::with_capacity(end);
    for &(c, color, bold) in &chars[..end] {
        placed.push(Placed { c, x, color, bold });
        x += font.advance(c, bold);
    }
    (placed, x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn compound(kv: &[(&str, Value)]) -> Value {
        Value::Compound(
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[test]
    fn flattens_every_stored_form() {
        assert_eq!(
            plain(&Value::String("Ferry".into()), &Lang::default()),
            "Ferry"
        );
        assert_eq!(
            plain(&Value::String(r#"{"text":"Old"}"#.into()), &Lang::default()),
            "Old"
        );
        assert_eq!(
            plain(&Value::String(r#""json""#.into()), &Lang::default()),
            "json"
        );
        let c = compound(&[
            ("text", Value::String("A".into())),
            ("color", Value::String("red".into())),
            (
                "extra",
                Value::List(vec![
                    Value::String("B".into()),
                    compound(&[
                        ("text", Value::String("C".into())),
                        ("color", Value::String("#123456".into())),
                    ]),
                ]),
            ),
        ]);
        let spans = flatten(&c, &Lang::default());
        assert_eq!(
            spans,
            vec![
                Span {
                    text: "AB".into(),
                    color: Some(0xFF5555),
                    bold: false
                },
                Span {
                    text: "C".into(),
                    color: Some(0x123456),
                    bold: false
                }
            ]
        );
        let l = Value::List(vec![Value::String("x".into()), Value::String("y".into())]);
        assert_eq!(plain(&l, &Lang::default()), "xy");
        // A translated line resolves through the resource packs' language files.
        let mut lang = Lang::default();
        lang.insert("delve.slate", "FERRY, %s");
        let t = compound(&[
            ("translate", Value::String("delve.slate".into())),
            ("with", Value::List(vec![Value::String("in chalk".into())])),
        ]);
        assert_eq!(plain(&t, &lang), "FERRY, in chalk");
        assert_eq!(plain(&t, &Lang::default()), "delve.slate");
        let j = Value::String(r#"{"translate":"delve.slate","with":["ok"]}"#.into());
        assert_eq!(plain(&j, &lang), "FERRY, ok");
    }

    #[test]
    fn long_lines_break_at_a_space() {
        let font = Font::default(); // every character advances 6
        let spans = vec![Span {
            text: "aaa bbb".into(),
            color: None,
            bold: false,
        }];
        let (p, w) = layout_line(&font, &spans, 30.0);
        assert_eq!(p.len(), 3);
        assert_eq!(w, 18.0);
        let (p, _) = layout_line(&font, &spans, 100.0);
        assert_eq!(p.len(), 7);
        let one = vec![Span {
            text: "abcdefgh".into(),
            color: None,
            bold: false,
        }];
        assert_eq!(layout_line(&font, &one, 30.0).0.len(), 5);
    }
}
