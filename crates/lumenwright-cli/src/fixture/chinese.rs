//! A text display and a sign that carry Chinese text, in front of two backdrops (dark
//! for the white display text, light for the black sign text).

use fastnbt::Value;

use super::{FixtureWorld, bs, props, s, text_side};

/// The text on both: "toll road", four characters from the Unifont.
pub const TEXT: &str = "收费之路";

/// Where the display stands; its text rises from here, centred on x.
pub const DISPLAY: [f64; 3] = [5.5, 66.0, 0.5];

/// The sign's block position; it faces south (+z).
pub const SIGN: [i32; 3] = [0, 65, 0];

/// The world with its text, or the same blocks with the display left out and the
/// sign's lines empty.
pub fn chinese_text_gallery(with_text: bool) -> FixtureWorld {
    let mut w = FixtureWorld::new();
    for x in -3..10 {
        for z in -3..7 {
            w.set(x, 63, z, bs("stone"));
            w.set(x, 64, z, bs("smooth_stone"));
        }
        for y in 65..69 {
            let backdrop = if x < 3 {
                "white_concrete"
            } else {
                "deepslate_bricks"
            };
            w.set(x, y, -1, bs(backdrop));
        }
    }
    w.set(
        SIGN[0],
        SIGN[1],
        SIGN[2],
        props("oak_sign", &[("rotation", "0"), ("waterlogged", "false")]),
    );
    let lines = if with_text {
        [s(""), s(TEXT), s(""), s("")]
    } else {
        [s(""), s(""), s(""), s("")]
    };
    w.block_entity(
        SIGN,
        "sign",
        vec![
            ("front_text", text_side(lines, "black", false)),
            (
                "back_text",
                text_side([s(""), s(""), s(""), s("")], "black", false),
            ),
            ("is_waxed", Value::Byte(0)),
        ],
    );
    if with_text {
        w.entity(
            "text_display",
            DISPLAY,
            0.0,
            vec![("text", s(TEXT)), ("background", Value::Int(0))],
        );
    }
    w.spawn = [0, 65, 4];
    w.player = Some(([0.5, 66.25, 3.5], 180.0, 0.0));
    w
}
