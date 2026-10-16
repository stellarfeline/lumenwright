//! The text-display gallery: one text display per property Lumenwright draws, each
//! alone in its own bay along +x, facing south (+z) in front of a stone backdrop.

use std::collections::HashMap;

use fastnbt::Value;

use super::{FixtureWorld, bs, compound, s};

/// One display of the gallery: its name and where it stands.
#[derive(Clone, Copy, Debug)]
pub struct TextDisplayCase {
    pub name: &'static str,
    /// The display's position; its text rises from here, centred on x.
    pub pos: [f64; 3],
}

const fn case(name: &'static str, bay: u32) -> TextDisplayCase {
    TextDisplayCase {
        name,
        pos: [bay as f64 * 5.0 + 0.5, 66.0, 0.5],
    }
}

/// The displays, one per bay, five blocks apart. A camera at `pos + (0, 0.25, 3)`
/// looking north sees one bay; `see_through` and `hidden` stand behind a wall.
pub const TEXT_DISPLAYS: &[TextDisplayCase] = &[
    case("align_left", 0),
    case("align_center", 1),
    case("align_right", 2),
    case("wrapped", 3),
    case("coloured_background", 4),
    case("default_background", 5),
    case("see_through", 6),
    case("hidden", 7),
    case("shadow", 8),
    case("no_shadow", 9),
    case("billboard_fixed", 10),
    case("billboard_center", 11),
    case("billboard_vertical", 12),
    case("billboard_horizontal", 13),
    case("transformed", 14),
    case("untransformed", 15),
    case("component", 16),
    case("component_plain", 17),
];

fn byte(b: bool) -> Value {
    Value::Byte(b as i8)
}

fn floats(v: &[f32]) -> Value {
    Value::List(v.iter().map(|f| Value::Float(*f)).collect())
}

/// A text component compound with a colour and bold.
fn styled_bold(t: &str, color: &str, bold: bool) -> Value {
    let mut c: HashMap<String, Value> = HashMap::new();
    c.insert("text".into(), s(t));
    c.insert("color".into(), s(color));
    c.insert("bold".into(), byte(bold));
    Value::Compound(c)
}

/// The fields of each display (beyond `id`, `Pos` and `Rotation`), and its yaw.
fn fields(name: &str) -> (f32, Vec<(&'static str, Value)>) {
    let two_lines = |t: &str| s(t);
    match name {
        "align_left" => (
            0.0,
            vec![
                ("text", two_lines("Left aligned\nshort")),
                ("alignment", s("left")),
            ],
        ),
        "align_center" => (0.0, vec![("text", two_lines("Centred text\nshort"))]),
        "align_right" => (
            0.0,
            vec![
                ("text", two_lines("Right aligned\nshort")),
                ("alignment", s("right")),
            ],
        ),
        "wrapped" => (
            0.0,
            vec![
                ("text", s("a line that wraps at forty pixels")),
                ("line_width", Value::Int(40)),
            ],
        ),
        // Opaque blue (ARGB 0xFF2050C0, an int in the save).
        "coloured_background" => (
            0.0,
            vec![
                ("text", s("Coloured")),
                ("background", Value::Int(0xFF20_50C0_u32 as i32)),
            ],
        ),
        "default_background" => (
            0.0,
            vec![("text", s("Default")), ("default_background", byte(true))],
        ),
        "see_through" => (
            0.0,
            vec![("text", s("Seen through")), ("see_through", byte(true))],
        ),
        "hidden" => (0.0, vec![("text", s("Seen through"))]),
        "shadow" => (
            0.0,
            vec![
                ("text", s("Shadowed")),
                ("shadow", byte(true)),
                ("background", Value::Int(0)),
            ],
        ),
        "no_shadow" => (
            0.0,
            vec![("text", s("Shadowed")), ("background", Value::Int(0))],
        ),
        // Turned to face east: only a billboard brings them round to a camera in
        // the south.
        "billboard_fixed" => (90.0, vec![("text", s("Billboard"))]),
        "billboard_center" => (
            90.0,
            vec![("text", s("Billboard")), ("billboard", s("center"))],
        ),
        "billboard_vertical" => (
            90.0,
            vec![("text", s("Billboard")), ("billboard", s("vertical"))],
        ),
        "billboard_horizontal" => (
            90.0,
            vec![("text", s("Billboard")), ("billboard", s("horizontal"))],
        ),
        // Twice the size, turned 30 degrees about the display's z axis.
        "transformed" => {
            let half = 15f32.to_radians();
            (
                0.0,
                vec![
                    ("text", s("Turned")),
                    (
                        "transformation",
                        compound(vec![
                            ("translation", floats(&[0.0, 0.0, 0.0])),
                            ("left_rotation", floats(&[0.0, 0.0, half.sin(), half.cos()])),
                            ("scale", floats(&[2.0, 2.0, 2.0])),
                            ("right_rotation", floats(&[0.0, 0.0, 0.0, 1.0])),
                        ]),
                    ),
                ],
            )
        }
        "untransformed" => (0.0, vec![("text", s("Turned"))]),
        // A text component (stored as NBT since 1.21.5): bold gold, then red.
        "component" => (
            0.0,
            vec![(
                "text",
                compound(vec![
                    ("text", s("Gold")),
                    ("color", s("gold")),
                    ("bold", byte(true)),
                    (
                        "extra",
                        Value::List(vec![styled_bold(" red", "red", false)]),
                    ),
                ]),
            )],
        ),
        "component_plain" => (
            0.0,
            vec![(
                "text",
                compound(vec![
                    ("text", s("Gold")),
                    ("color", s("gold")),
                    (
                        "extra",
                        Value::List(vec![styled_bold(" red", "red", false)]),
                    ),
                ]),
            )],
        ),
        other => panic!("no text display case {other}"),
    }
}

/// The gallery of [`TEXT_DISPLAYS`]; without displays, the same blocks alone.
pub fn text_display_gallery(with_displays: bool) -> FixtureWorld {
    let mut w = FixtureWorld::new();
    let bays = TEXT_DISPLAYS.len() as i32;
    let (x0, x1) = (-3, bays * 5 + 2);
    for x in x0..x1 {
        for z in -3..7 {
            w.set(x, 63, z, bs("stone"));
            w.set(x, 64, z, bs("smooth_stone"));
        }
        // The backdrop.
        for y in 65..69 {
            w.set(x, y, -1, bs("stone_bricks"));
        }
    }
    for c in TEXT_DISPLAYS {
        if matches!(c.name, "shadow" | "no_shadow") {
            // A light backdrop, so the dark shadow shows.
            let bx = c.pos[0].floor() as i32;
            for x in bx - 2..=bx + 2 {
                for y in 65..69 {
                    w.set(x, y, -1, bs("light_gray_concrete"));
                }
            }
        }
        if matches!(c.name, "see_through" | "hidden") {
            // A wall between the display and a camera in the south.
            let bx = c.pos[0].floor() as i32;
            for x in bx - 2..=bx + 2 {
                for y in 65..68 {
                    w.set(x, y, 1, bs("stone_bricks"));
                }
            }
        }
        if with_displays {
            let (yaw, extra) = fields(c.name);
            w.entity("text_display", c.pos, yaw, extra);
        }
    }
    w.spawn = [0, 65, 4];
    w.player = Some(([0.5, 66.25, 3.5], 180.0, 0.0));
    w
}
