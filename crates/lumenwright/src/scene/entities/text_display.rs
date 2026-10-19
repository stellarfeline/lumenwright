//! Text displays, as `DisplayRenderer.TextDisplayRenderer` draws them: the `text`
//! component split into lines no wider than `line_width`, aligned left, right or
//! centred inside the widest line, over a background rectangle, with an optional drop
//! shadow, posed by the display's `billboard` orientation and `transformation`.
//! A `see_through` display is drawn over everything in front of it.
//!
//! Layout and placement happen before the atlas is packed (glyph sheets and the
//! background colours are textures); the quads are made once it exists.

use glam::{Mat4, Vec3};

use super::display_transform;
use crate::assets::atlas::AtlasBuilder;
use crate::assets::font::Font;
use crate::assets::lang::{DEFAULT_LANGUAGE, Lang};
use crate::assets::source::ResourceStore;
use crate::scene::text::{self, GlyphQuad};
use crate::scene::{Alpha, EntityLight, Quad, Scene};
use crate::world::{Entity, nbt};

/// `Display.TextDisplay` defaults.
const DEFAULT_LINE_WIDTH: f64 = 200.0;
const DEFAULT_BACKGROUND: u32 = 0x4000_0000;
/// `Options.getBackgroundOpacity(0.25)` with the default "background for chat only".
const DEFAULT_BACKGROUND_OPTION: u32 = ((0.25f32 * 255.0) as u32) << 24;
/// `font.lineHeight + 1`.
const LINE_HEIGHT: f32 = 10.0;
/// Text-space units per block (`scale(-0.025)`).
const TEXT_SCALE: f32 = 0.025;
/// Text-space distance of the shadow and the text in front of the background, so a
/// depth buffer and a ray both meet them first (vanilla uses a polygon offset).
const SHADOW_Z: f32 = 0.4;
const TEXT_Z: f32 = 0.8;
/// Size of a generated background texture.
const BACKGROUND_TEXELS: u32 = 4;

/// One quad of a text display, in scene coordinates.
#[derive(Clone, Copy, Debug)]
pub struct TextQuad {
    pub quad: Quad,
    /// Lit like the display: its block's light, no directional shading.
    pub light: EntityLight,
    /// Drawn over everything (`see_through`): no depth test in quick mode, and the
    /// camera ray meets it whatever lies in front of it in HQ mode.
    pub see_through: bool,
    /// The display's position; see-through quads are drawn far to near by it, each
    /// display's quads in the order they were made (background, shadow, text).
    pub anchor: Vec3,
}

/// A quad waiting for the atlas.
struct PlannedQuad {
    texture: String,
    tex_size: [f32; 2],
    uv: [f32; 4],
    p0: Vec3,
    eu: Vec3,
    ev: Vec3,
    rgb: u32,
    alpha: Alpha,
}

struct Planned {
    quads: Vec<PlannedQuad>,
    light: [i32; 3],
    see_through: bool,
    anchor: Vec3,
}

/// Every text display in the scene, laid out and placed.
#[derive(Default)]
pub struct TextDisplayPlan {
    planned: Vec<Planned>,
    textures: Vec<String>,
    /// Background colours (ARGB) that need a generated texture.
    backgrounds: Vec<u32>,
    /// Glyph quads planned, for the log.
    pub glyphs: usize,
}

fn background_texture(argb: u32) -> String {
    format!("lumenwright:text_background/{argb:08x}")
}

impl TextDisplayPlan {
    pub fn register_textures(&self, ab: &mut AtlasBuilder) {
        for t in &self.textures {
            ab.add(t);
        }
        for &argb in &self.backgrounds {
            // Atlas texels are RGBA8 with red in the low byte.
            let [b, g, r, a] = argb.to_le_bytes();
            let texel = u32::from_le_bytes([r, g, b, a]);
            let n = BACKGROUND_TEXELS;
            ab.add_generated(
                &background_texture(argb),
                n,
                n,
                vec![texel; (n * n) as usize],
            );
        }
    }

    pub fn is_empty(&self) -> bool {
        self.planned.is_empty()
    }
}

/// `Display.TextDisplay.getAlign` on the stored `alignment`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

fn align(e: &Entity) -> Align {
    match e.str_field("alignment") {
        Some("left") => Align::Left,
        Some("right") => Align::Right,
        _ => Align::Center,
    }
}

/// The background colour (ARGB) a display draws, 0 for none.
fn background(e: &Entity) -> u32 {
    if e.flag("default_background") {
        return DEFAULT_BACKGROUND_OPTION;
    }
    nbt::get(&e.nbt, "background")
        .and_then(nbt::as_i64)
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_BACKGROUND)
}

/// Lays out and places every text display. `origin` and `size` are the scene's
/// (blocks); `view` is the camera's yaw and pitch, which billboards turn to.
pub fn plan(
    store: &ResourceStore,
    displays: &[Entity],
    origin: [i32; 3],
    size: [i32; 3],
    view: Option<[f32; 2]>,
) -> TextDisplayPlan {
    let mut out = TextDisplayPlan::default();
    if displays.is_empty() {
        return out;
    }
    let font = Font::load(store);
    let lang = Lang::load(store, DEFAULT_LANGUAGE);
    for e in displays {
        let rel = Vec3::new(
            (e.pos[0] - origin[0] as f64) as f32,
            (e.pos[1] - origin[1] as f64) as f32,
            (e.pos[2] - origin[2] as f64) as f32,
        );
        if rel.x < 0.0 || rel.z < 0.0 || rel.x >= size[0] as f32 || rel.z >= size[2] as f32 {
            continue;
        }
        let quads = place(&font, &lang, e, rel, view, &mut out);
        let lp = rel.floor();
        out.planned.push(Planned {
            quads,
            light: [lp.x as i32, lp.y as i32, lp.z as i32],
            see_through: e.flag("see_through"),
            anchor: rel,
        });
    }
    out.textures.sort();
    out.textures.dedup();
    out.backgrounds.sort();
    out.backgrounds.dedup();
    log::debug!(
        "text displays: {} planned, {} glyph quads",
        out.planned.len(),
        out.glyphs
    );
    out
}

/// `TextDisplayRenderer.renderInner` for one display.
fn place(
    font: &Font,
    lang: &Lang,
    e: &Entity,
    rel: Vec3,
    view: Option<[f32; 2]>,
    out: &mut TextDisplayPlan,
) -> Vec<PlannedQuad> {
    let spans = e
        .nbt
        .get("text")
        .map(|v| text::flatten(v, lang))
        .unwrap_or_default();
    let line_width = e.num_field("line_width").unwrap_or(DEFAULT_LINE_WIDTH) as f32;
    let lines = text::layout_lines(font, &spans, line_width);
    // CachedInfo.width: the widest line (`Font.width`, rounded up).
    let width = lines.iter().map(|(_, w)| w.ceil()).fold(0.0, f32::max);
    let height = lines.len() as f32 * LINE_HEIGHT;
    // The display's pose, then rotate(pi about y), scale(-0.025), and the text block
    // centred on x with its last line ending at the display's origin.
    let m = Mat4::from_translation(rel)
        * display_transform(e, view)
        * Mat4::from_rotation_y(std::f32::consts::PI)
        * Mat4::from_scale(Vec3::splat(-TEXT_SCALE))
        * Mat4::from_translation(Vec3::new(1.0 - width / 2.0, -height, 0.0));
    let mut quads = Vec::new();
    let argb = background(e);
    if argb != 0 {
        let tex = background_texture(argb);
        if !out.backgrounds.contains(&argb) {
            out.backgrounds.push(argb);
        }
        let n = BACKGROUND_TEXELS as f32;
        quads.push(PlannedQuad {
            texture: tex,
            tex_size: [n, n],
            uv: [0.0, 0.0, n, n],
            p0: m.transform_point3(Vec3::new(-1.0, -1.0, 0.0)),
            eu: m.transform_vector3(Vec3::new(width + 1.0, 0.0, 0.0)),
            ev: m.transform_vector3(Vec3::new(0.0, height + 1.0, 0.0)),
            rgb: 0xFF_FFFF,
            alpha: if argb >> 24 == 0xFF {
                Alpha::Opaque
            } else {
                Alpha::Translucent
            },
        });
    }
    let shadow = e.flag("shadow");
    let mut glyphs: Vec<GlyphQuad> = Vec::new();
    for (i, (placed, w)) in lines.iter().enumerate() {
        let x = match align(e) {
            Align::Left => 0.0,
            Align::Right => width - w.ceil(),
            Align::Center => width / 2.0 - w.ceil() / 2.0,
        };
        let y = i as f32 * LINE_HEIGHT;
        if shadow {
            // Font.drawInBatch: the shadow one pixel right and down, at a quarter of
            // the colour, then the text.
            let o = Vec3::new(x + 1.0, y + 1.0, SHADOW_Z);
            text::glyph_quads(font, placed, o, 0xFF_FFFF, false, 0.25, &mut glyphs);
        }
        let o = Vec3::new(x, y, TEXT_Z);
        text::glyph_quads(font, placed, o, 0xFF_FFFF, false, 1.0, &mut glyphs);
    }
    out.glyphs += glyphs.len();
    for g in glyphs {
        out.textures.push(g.texture.clone());
        quads.push(PlannedQuad {
            texture: g.texture,
            tex_size: g.tex_size,
            uv: g.uv,
            p0: m.transform_point3(g.p0),
            eu: m.transform_vector3(g.eu),
            ev: m.transform_vector3(g.ev),
            rgb: g.rgb,
            alpha: Alpha::Cutout,
        });
    }
    quads
}

/// Makes the planned quads (after the atlas is packed) and counts the displays as
/// rendered entities.
pub fn build_quads(scene: &mut Scene, plan: &TextDisplayPlan) {
    for p in &plan.planned {
        let light = EntityLight {
            pos: p.light,
            shade: false,
            full_bright: false,
        };
        for q in &p.quads {
            let sprite = scene.atlas.sprite(&q.texture);
            let a = sprite.to_atlas(q.uv[0], q.uv[1], q.tex_size);
            let b = sprite.to_atlas(q.uv[2], q.uv[1], q.tex_size);
            let c = sprite.to_atlas(q.uv[0], q.uv[3], q.tex_size);
            let rgb = q.rgb;
            scene.text_quads.push(TextQuad {
                quad: Quad {
                    p0: q.p0,
                    eu: q.eu,
                    ev: q.ev,
                    uv0: a,
                    uvs: [b[0] - a[0], b[1] - a[1]],
                    uvt: [c[0] - a[0], c[1] - a[1]],
                    tint: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255],
                    alpha: q.alpha,
                    water: false,
                    shade: false,
                    emission: 0.0,
                    cullface: None,
                    ao: false,
                },
                light,
                see_through: p.see_through,
                anchor: p.anchor,
            });
        }
    }
    if !plan.planned.is_empty() {
        *scene
            .entity_stats
            .rendered
            .entry("text_display".to_string())
            .or_default() += plan.planned.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastnbt::Value;

    fn display(extra: &[(&str, Value)]) -> Entity {
        let mut c = nbt::Compound::new();
        for (k, v) in extra {
            c.insert(k.to_string(), v.clone());
        }
        Entity {
            id: "minecraft:text_display".into(),
            pos: [0.5, 1.0, 0.5],
            yaw: 0.0,
            pitch: 0.0,
            nbt: c,
        }
    }

    #[test]
    fn background_follows_the_flags() {
        assert_eq!(background(&display(&[])), 0x4000_0000);
        assert_eq!(
            background(&display(&[("background", Value::Int(0x7F00_FF00))])),
            0x7F00_FF00
        );
        assert_eq!(
            background(&display(&[("background", Value::Int(-16_777_216))])),
            0xFF00_0000
        );
        assert_eq!(
            background(&display(&[("default_background", Value::Byte(1))])),
            0x3F00_0000
        );
        assert_eq!(align(&display(&[])), Align::Center);
        assert_eq!(
            align(&display(&[("alignment", Value::String("left".into()))])),
            Align::Left
        );
    }

    #[test]
    fn a_fixed_display_faces_south_above_its_origin() {
        // With no font every character advances 6 and draws nothing: only the
        // background is placed. At yaw 0 it faces +z and sits on its origin.
        let e = display(&[("text", Value::String("abcd".into()))]);
        let mut plan = TextDisplayPlan::default();
        let q = place(
            &Font::default(),
            &Lang::default(),
            &e,
            Vec3::new(0.5, 1.0, 0.5),
            None,
            &mut plan,
        );
        assert_eq!(q.len(), 1);
        let bg = &q[0];
        let n = bg.ev.cross(bg.eu).normalize();
        assert!((n - Vec3::Z).length() < 1e-5, "normal {n}");
        // 24 px wide plus the 1 px margin each side, 10 px tall plus 1, at 0.025.
        assert!((bg.eu.length() - 25.0 * TEXT_SCALE).abs() < 1e-5);
        assert!((bg.ev.length() - 11.0 * TEXT_SCALE).abs() < 1e-5);
        let ys: Vec<f32> = [bg.p0, bg.p0 + bg.ev].iter().map(|p| p.y).collect();
        assert!(
            (ys[1] - 1.0).abs() < 1e-5,
            "bottom edge at the origin: {ys:?}"
        );
        assert!(ys[0] > ys[1]);
        let xs = [bg.p0.x, (bg.p0 + bg.eu).x];
        assert!(xs[0] < xs[1], "text runs towards +x seen from the south");
        assert!(
            ((xs[0] + xs[1]) / 2.0 - 0.5).abs() < 0.02,
            "centred: {xs:?}"
        );
    }
}
