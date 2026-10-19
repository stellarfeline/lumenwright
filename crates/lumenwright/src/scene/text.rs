//! Text components as stored in a save (sign messages, text displays), flattened to
//! styled runs, laid out with the vanilla bitmap font and turned into glyph quads.

use fastnbt::Value;
use glam::Vec3;

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
    /// Drawn in the italic face (sheared a quarter pixel per pixel of height).
    pub italic: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct Style {
    color: Option<u32>,
    bold: bool,
    italic: bool,
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
        && last.italic == st.italic
    {
        last.text.push_str(text);
        return;
    }
    out.push(Span {
        text: text.to_string(),
        color: st.color,
        bold: st.bold,
        italic: st.italic,
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
        italic: match c.get("italic") {
            Some(v) => nbt::as_i64(v).unwrap_or(0) != 0,
            None => parent.italic,
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
        italic: o
            .get("italic")
            .and_then(|b| b.as_bool())
            .unwrap_or(parent.italic),
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
    /// Drawn in the italic face.
    pub italic: bool,
}

/// Lays out one line and returns its characters and width. Like the sign renderer
/// (`Font.split(line, maxWidth)`, first line kept), text past `max_width` is cut at
/// the last space that fits, or mid-word when no space does.
pub fn layout_line(font: &Font, spans: &[Span], max_width: f32) -> (Vec<Placed>, f32) {
    let chars: Vec<StyledChar> = styled_chars(spans).take_while(|s| s.c != '\n').collect();
    let mut width = 0.0;
    let mut end = chars.len();
    let mut last_space: Option<usize> = None;
    for (i, &StyledChar { c, bold, .. }) in chars.iter().enumerate() {
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
    place(font, &chars[..end])
}

#[derive(Clone, Copy)]
struct StyledChar {
    c: char,
    color: Option<u32>,
    bold: bool,
    italic: bool,
}

fn styled_chars(spans: &[Span]) -> impl Iterator<Item = StyledChar> + '_ {
    spans.iter().flat_map(|s| {
        s.text.chars().map(move |c| StyledChar {
            c,
            color: s.color,
            bold: s.bold,
            italic: s.italic,
        })
    })
}

fn place(font: &Font, chars: &[StyledChar]) -> (Vec<Placed>, f32) {
    let mut x = 0.0;
    let mut placed = Vec::with_capacity(chars.len());
    for s in chars {
        placed.push(Placed {
            c: s.c,
            x,
            color: s.color,
            bold: s.bold,
            italic: s.italic,
        });
        x += font.advance(s.c, s.bold);
    }
    (placed, x)
}

/// Lays out text as `Font.split(text, max_width)` does (`StringSplitter.splitLines`):
/// a line ends at `\n`, or where the next character would pass `max_width`, at the
/// last space when the line has one and mid-word when it does not; the newline or
/// space a line ends at is dropped. Every line keeps at least one character.
/// Returns each line's characters and width.
pub fn layout_lines(font: &Font, spans: &[Span], max_width: f32) -> Vec<(Vec<Placed>, f32)> {
    let chars: Vec<StyledChar> = styled_chars(spans).collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        // LineBreakFinder.accept: a space is a break candidate before it is measured.
        let mut width = 0.0;
        let mut last_space: Option<usize> = None;
        let mut had_width = false;
        // (end of the line, characters skipped after it)
        let mut cut = (chars.len(), 0);
        for (i, s) in chars.iter().enumerate().skip(start) {
            if s.c == '\n' {
                cut = (i, 1);
                break;
            }
            if s.c == ' ' {
                last_space = Some(i);
            }
            let w = font.advance(s.c, s.bold);
            width += w;
            if had_width && width > max_width {
                cut = match last_space {
                    Some(sp) => (sp, 1),
                    None => (i, 0),
                };
                break;
            }
            had_width |= w != 0.0;
        }
        lines.push(place(font, &chars[start..cut.0]));
        start = cut.0 + cut.1;
    }
    lines
}

/// One glyph quad in text space (x right, y down, in font pixels).
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphQuad {
    /// Texture of the glyph page, namespaced.
    pub texture: String,
    /// Size of that texture in pixels (width, height).
    pub tex_size: [f32; 2],
    /// u0, v0, u1, v1 in texture pixels.
    pub uv: [f32; 4],
    /// Top-left corner in text space (x right, y down, font pixels).
    pub p0: Vec3,
    /// Edge from `p0` along the glyph's width.
    pub eu: Vec3,
    /// Edge from `p0` along the glyph's height, including the italic shear.
    pub ev: Vec3,
    /// RGB colour (0xRRGGBB) after shading.
    pub rgb: u32,
}

/// Scales each channel of an RGB colour (`ARGB.scaleRGB`).
pub fn scale_rgb(c: u32, f: f32) -> u32 {
    let ch = |s: u32| ((((c >> s) & 0xFF) as f32 * f) as u32) << s;
    ch(16) | ch(8) | ch(0)
}

/// The glyph quads of one laid-out line whose top-left is `origin` (text space), as
/// `BakedGlyph.render` places them: bold draws the glyph again one pixel right, italic
/// shears the top right and the bottom left by a quarter pixel per pixel of height.
/// `color` is the colour of characters with none of their own; `forced` uses it for
/// every character (sign outlines); `shade` scales the result (0.25 for a shadow).
pub fn glyph_quads(
    font: &Font,
    placed: &[Placed],
    origin: Vec3,
    color: u32,
    forced: bool,
    shade: f32,
    out: &mut Vec<GlyphQuad>,
) {
    for p in placed {
        let Some(g) = font.glyph(p.c) else {
            continue;
        };
        let rgb = if forced {
            color
        } else {
            p.color.unwrap_or(color)
        };
        let rgb = if shade == 1.0 {
            rgb
        } else {
            scale_rgb(rgb, shade)
        };
        let w = g.cell[2] * g.scale;
        let h = g.cell[3] * g.scale;
        let up = 7.0 - g.ascent;
        let (top_shear, bottom_shear) = if p.italic {
            (1.0 - 0.25 * up, 1.0 - 0.25 * (up + h))
        } else {
            (0.0, 0.0)
        };
        let copies: &[f32] = if p.bold { &[0.0, 1.0] } else { &[0.0] };
        for &bx in copies {
            let left = origin.x + p.x + bx;
            out.push(GlyphQuad {
                texture: g.texture.clone(),
                tex_size: g.texture_size,
                uv: [
                    g.cell[0],
                    g.cell[1],
                    g.cell[0] + g.cell[2],
                    g.cell[1] + g.cell[3],
                ],
                p0: Vec3::new(left + top_shear, origin.y + up, origin.z),
                eu: Vec3::new(w, 0.0, 0.0),
                ev: Vec3::new(bottom_shear - top_shear, h, 0.0),
                rgb,
            });
        }
    }
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
                    bold: false,
                    italic: false
                },
                Span {
                    text: "C".into(),
                    color: Some(0x123456),
                    bold: false,
                    italic: false
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
            italic: false,
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
            italic: false,
        }];
        assert_eq!(layout_line(&font, &one, 30.0).0.len(), 5);
    }

    #[test]
    fn lines_split_like_string_splitter() {
        let font = Font::default(); // every character advances 6
        let span = |t: &str| {
            vec![Span {
                text: t.into(),
                color: None,
                bold: false,
                italic: false,
            }]
        };
        let text = |lines: Vec<(Vec<Placed>, f32)>| -> Vec<String> {
            lines
                .into_iter()
                .map(|(p, _)| p.iter().map(|c| c.c).collect())
                .collect()
        };
        // A newline ends a line; the space a line breaks at is dropped.
        assert_eq!(
            text(layout_lines(&font, &span("aaa bbb\ncc"), 30.0)),
            ["aaa", "bbb", "cc"]
        );
        // The space that passes the width is itself the break.
        assert_eq!(
            text(layout_lines(&font, &span("aaaaa bb"), 30.0)),
            ["aaaaa", "bb"]
        );
        // No space: mid-word; a line keeps one character however narrow.
        assert_eq!(
            text(layout_lines(&font, &span("abcdefg"), 30.0)),
            ["abcde", "fg"]
        );
        assert_eq!(text(layout_lines(&font, &span("ab"), 1.0)), ["a", "b"]);
        let (_, w) = &layout_lines(&font, &span("aaa bbb"), 100.0)[0];
        assert_eq!(*w, 42.0);
        assert!(layout_lines(&font, &span(""), 100.0).is_empty());
    }
}
