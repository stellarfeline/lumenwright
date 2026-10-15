//! Block entities whose look vanilla draws in Java code (`BlockEntityRenderer`s):
//! signs and hanging signs with their text, bells, lectern and enchanting-table
//! books, chests, banners, decorated pots, skulls and heads.
//!
//! Geometry is data (`assets/block_entity_models.json`, the same cuboid and box-UV
//! format as the entity models); the per-block placement below transcribes each
//! renderer's `PoseStack` calls from the 1.21.11 client. The quads join the entity
//! quads, so quick and HQ mode, and every backend, draw the same geometry.

use std::collections::{BTreeMap, HashMap};

use fastnbt::Value;
use glam::{Mat4, Vec3};
use serde::Deserialize;

use super::entities::{ModelDef, PartPose, part_quads};
use super::text::{self, Span};
use super::{Alpha, EntityLight, Quad, Scene};
use crate::assets::ResourceStore;
use crate::assets::atlas::{AtlasBuilder, load_texture};
use crate::assets::font::Font;
use crate::assets::lang::{DEFAULT_LANGUAGE, Lang};
use crate::assets::source::{norm_loc, split_loc};
use crate::world::nbt::{self, Compound};
use crate::world::{BlockState, BlockVolume, Registry};

const BUILTIN_MODELS: &str = include_str!("../../assets/block_entity_models.json");

#[derive(Deserialize)]
pub struct BlockEntityModels {
    pub format: u32,
    pub models: HashMap<String, ModelDef>,
}

impl BlockEntityModels {
    pub fn load_builtin() -> BlockEntityModels {
        serde_json::from_str(BUILTIN_MODELS).expect("bundled block_entity_models.json is valid")
    }
}

/// A block entity record from a chunk.
#[derive(Clone, Debug)]
pub struct BlockEntity {
    /// Namespaced id, e.g. `minecraft:sign`.
    pub id: String,
    pub pos: [i32; 3],
    pub nbt: Compound,
}

impl BlockEntity {
    pub fn from_nbt(v: &Value) -> Option<BlockEntity> {
        let c = nbt::compound(v)?;
        let id = norm_loc(nbt::get_str(c, "id")?);
        let p = |k: &str| nbt::get(c, k).and_then(nbt::as_i64).map(|v| v as i32);
        Some(BlockEntity {
            id,
            pos: [p("x")?, p("y")?, p("z")?],
            nbt: c.clone(),
        })
    }

    pub fn short_id(&self) -> &str {
        self.id.strip_prefix("minecraft:").unwrap_or(&self.id)
    }

    /// The front and back text of a sign, one string per line (for `info`).
    pub fn sign_lines(&self, lang: &Lang) -> Option<[Vec<String>; 2]> {
        let side = |k: &str| -> Vec<String> {
            nbt::get(&self.nbt, &format!("{k}.messages"))
                .and_then(|m| match m {
                    Value::List(l) => Some(l.iter().map(|m| text::plain(m, lang)).collect()),
                    _ => None,
                })
                .unwrap_or_default()
        };
        matches!(self.short_id(), "sign" | "hanging_sign")
            .then(|| [side("front_text"), side("back_text")])
    }
}

pub fn parse_all(values: &[Value]) -> Vec<BlockEntity> {
    let mut out: Vec<BlockEntity> = values.iter().filter_map(BlockEntity::from_nbt).collect();
    out.sort_by_key(|b| (b.pos[1], b.pos[2], b.pos[0]));
    out
}

#[derive(Default, Debug, Clone)]
pub struct BlockEntityStats {
    /// Drawn block entities by kind (`sign`, `bell`, `chest`...).
    pub rendered: BTreeMap<String, usize>,
    /// Block entities with no Java-drawn look, or whose block does not match.
    pub skipped: BTreeMap<String, usize>,
    /// Sign lines with text, and glyphs drawn for them.
    pub text_lines: usize,
    pub glyphs: usize,
}

/// One piece of geometry to emit once the atlas exists.
enum Draw {
    Model {
        model: &'static str,
        texture: String,
        m: Mat4,
        poses: Vec<(&'static str, PoseEdit)>,
        /// Only these top-level parts are drawn (all when empty).
        only: Vec<&'static str>,
    },
    Glyph {
        texture: String,
        tex_size: [f32; 2],
        /// u0, v0, u1, v1 in texture pixels.
        uv: [f32; 4],
        p0: Vec3,
        eu: Vec3,
        ev: Vec3,
        rgb: u32,
        full_bright: bool,
    },
}

#[derive(Clone, Copy, Default)]
struct PoseEdit {
    pivot_x: Option<f32>,
    rotation: [Option<f32>; 3],
}

impl PoseEdit {
    fn rot(x: Option<f32>, y: Option<f32>, z: Option<f32>) -> PoseEdit {
        PoseEdit {
            pivot_x: None,
            rotation: [x, y, z],
        }
    }
}

struct Planned {
    /// Block position relative to the scene origin.
    rel: [i32; 3],
    draws: Vec<Draw>,
}

/// Everything planned before the atlas is packed: the draws and the textures they
/// need (including composited banner textures).
pub struct BlockEntityPlan {
    planned: Vec<Planned>,
    textures: Vec<String>,
    generated: Vec<(String, u32, u32, Vec<u32>)>,
    pub stats: BlockEntityStats,
}

impl BlockEntityPlan {
    pub fn register_textures(&mut self, ab: &mut AtlasBuilder) {
        for t in &self.textures {
            ab.add(t);
        }
        for (loc, w, h, px) in self.generated.drain(..) {
            ab.add_generated(&loc, w, h, px);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.planned.is_empty()
    }
}

/// Context for planning: where blocks are and who looks at them.
pub struct PlanContext<'a> {
    pub store: &'a ResourceStore,
    pub registry: &'a Registry,
    pub volume: &'a BlockVolume,
    pub origin: [i32; 3],
    /// Camera position in world coordinates (glowing-text outlines are drawn within
    /// 16 blocks, as in `AbstractSignRenderer`).
    pub eye: Option<[f64; 3]>,
}

pub fn plan(ctx: &PlanContext, list: &[BlockEntity]) -> BlockEntityPlan {
    let mut font: Option<(Font, Lang)> = None;
    let mut out = BlockEntityPlan {
        planned: Vec::new(),
        textures: Vec::new(),
        generated: Vec::new(),
        stats: BlockEntityStats::default(),
    };
    let size = ctx.volume.size_blocks();
    for be in list {
        let rel = [
            be.pos[0] - ctx.origin[0],
            be.pos[1] - ctx.origin[1],
            be.pos[2] - ctx.origin[2],
        ];
        if (0..3).any(|i| rel[i] < 0 || rel[i] >= size[i]) {
            continue;
        }
        let state = ctx
            .registry
            .state(ctx.volume.block(be.pos[0], be.pos[1], be.pos[2]));
        let t = Mat4::from_translation(Vec3::new(rel[0] as f32, rel[1] as f32, rel[2] as f32));
        let mut draws = Vec::new();
        let kind = match be.short_id() {
            "sign" | "hanging_sign" => {
                let (font, lang) = font.get_or_insert_with(|| {
                    (
                        Font::load(ctx.store),
                        Lang::load(ctx.store, DEFAULT_LANGUAGE),
                    )
                });
                plan_sign(ctx, be, state, t, font, lang, &mut out.stats, &mut draws)
            }
            "bell" => plan_bell(state, t, &mut draws),
            "chest" | "trapped_chest" | "ender_chest" | "copper_chest" => {
                plan_chest(state, t, &mut draws)
            }
            "lectern" => plan_lectern(state, t, &mut draws),
            "enchanting_table" => plan_enchanting_table(state, t, &mut draws),
            "banner" => plan_banner(ctx, be, state, t, &mut out.generated, &mut draws),
            "decorated_pot" => plan_decorated_pot(be, state, t, &mut draws),
            "skull" => plan_skull(ctx, be, state, t, &mut draws),
            _ => None,
        };
        match kind {
            Some(kind) => {
                for d in &draws {
                    if let Draw::Model { texture, .. } | Draw::Glyph { texture, .. } = d
                        && !texture.starts_with("lumenwright:")
                    {
                        out.textures.push(texture.clone());
                    }
                }
                *out.stats.rendered.entry(kind.to_string()).or_default() += 1;
                out.planned.push(Planned { rel, draws });
            }
            None => {
                *out.stats
                    .skipped
                    .entry(be.short_id().to_string())
                    .or_default() += 1;
            }
        }
    }
    out.textures.sort();
    out.textures.dedup();
    if !out.stats.rendered.is_empty() {
        log::info!("block entities rendered: {:?}", out.stats.rendered);
    }
    if !out.stats.skipped.is_empty() {
        log::debug!(
            "block entities without a drawn model: {:?}",
            out.stats.skipped
        );
    }
    out
}

/// `Direction.toYRot()` of a horizontal facing name.
fn facing_yrot(f: &str) -> f32 {
    match f {
        "south" => 0.0,
        "west" => 90.0,
        "north" => 180.0,
        "east" => 270.0,
        _ => 0.0,
    }
}

fn facing_step(f: &str) -> (f32, f32) {
    match f {
        "north" => (0.0, -1.0),
        "south" => (0.0, 1.0),
        "west" => (-1.0, 0.0),
        "east" => (1.0, 0.0),
        _ => (0.0, 0.0),
    }
}

fn rotation_degrees(state: &BlockState) -> f32 {
    // RotationSegment.convertToDegrees: 16 segments of 22.5 degrees.
    state
        .prop("rotation")
        .and_then(|r| r.parse::<f32>().ok())
        .unwrap_or(0.0)
        * 22.5
}

fn tr(x: f32, y: f32, z: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(x, y, z))
}

fn rot_y_deg(d: f32) -> Mat4 {
    Mat4::from_rotation_y(d.to_radians())
}

fn model(texture: &str, model: &'static str, m: Mat4) -> Draw {
    Draw::Model {
        model,
        texture: norm_loc(texture),
        m,
        poses: Vec::new(),
        only: Vec::new(),
    }
}

/// The wood of a sign block (`spruce_wall_hanging_sign` -> `spruce`).
fn sign_wood(name: &str) -> Option<&str> {
    for suffix in ["_wall_hanging_sign", "_hanging_sign", "_wall_sign", "_sign"] {
        if let Some(w) = name.strip_suffix(suffix) {
            return Some(w);
        }
    }
    None
}

/// DyeColor `textColor` by name.
fn dye_text_color(name: &str) -> u32 {
    match name {
        "white" => 0xFFFFFF,
        "orange" => 0xFF681F,
        "magenta" => 0xFF00FF,
        "light_blue" => 0x9AC0CD,
        "yellow" => 0xFFFF00,
        "lime" => 0xBFFF00,
        "pink" => 0xFF69B4,
        "gray" => 0x808080,
        "light_gray" => 0xD3D3D3,
        "cyan" => 0x00FFFF,
        "purple" => 0xA020F0,
        "blue" => 0x0000FF,
        "brown" => 0x8B4513,
        "green" => 0x00FF00,
        "red" => 0xFF0000,
        _ => 0x000000,
    }
}

/// DyeColor `textureDiffuseColor` by name.
pub fn dye_diffuse_color(name: &str) -> Option<u32> {
    Some(match name {
        "white" => 0xF9FFFE,
        "orange" => 0xF9801D,
        "magenta" => 0xC74EBD,
        "light_blue" => 0x3AB3DA,
        "yellow" => 0xFED83D,
        "lime" => 0x80C71F,
        "pink" => 0xF38BAA,
        "gray" => 0x474F52,
        "light_gray" => 0x9D9D97,
        "cyan" => 0x169C9C,
        "purple" => 0x8932B8,
        "blue" => 0x3C44AA,
        "brown" => 0x835432,
        "green" => 0x5E7C16,
        "red" => 0xB02E26,
        "black" => 0x1D1D21,
        _ => return None,
    })
}

/// `ARGB.scaleRGB(color, f)`.
fn scale_rgb(c: u32, f: f32) -> u32 {
    let ch = |s: u32| ((((c >> s) & 0xFF) as f32 * f) as u32) << s;
    ch(16) | ch(8) | ch(0)
}

#[allow(clippy::too_many_arguments)]
fn plan_sign(
    ctx: &PlanContext,
    be: &BlockEntity,
    state: &BlockState,
    t: Mat4,
    font: &Font,
    lang: &Lang,
    stats: &mut BlockEntityStats,
    draws: &mut Vec<Draw>,
) -> Option<&'static str> {
    let name = state.short_name();
    let wood = sign_wood(name)?;
    let hanging = name.ends_with("hanging_sign");
    let wall = name.contains("_wall_");
    let yrot = if wall {
        facing_yrot(state.prop("facing").unwrap_or("north"))
    } else {
        rotation_degrees(state)
    };
    // (base pose, model, model scale, text scale, text offset, line height, max width)
    let (base, model_name, model_scale, text_scale, offset, line_h, max_w) = if hanging {
        // HangingSignRenderer.translateBase, createHangingSignLayer, getSignModelRenderScale 1,
        // getSignTextRenderScale 0.9, TEXT_OFFSET, HangingSignBlockEntity line metrics.
        let model_name = if wall {
            "hanging_sign_wall"
        } else if state.prop("attached") == Some("true") {
            "hanging_sign_ceiling_middle"
        } else {
            "hanging_sign_ceiling"
        };
        (
            t * tr(0.5, 0.9375, 0.5) * rot_y_deg(-yrot) * tr(0.0, -0.3125, 0.0),
            model_name,
            Vec3::new(1.0, -1.0, -1.0),
            0.015625 * 0.9,
            Vec3::new(0.0, -0.32, 0.073),
            9.0,
            60.0,
        )
    } else {
        // SignRenderer: translate(0.5) rotate, wall offset; model scale 2/3 (y, z flipped).
        let mut base = t * tr(0.5, 0.5, 0.5) * rot_y_deg(-yrot);
        if wall {
            base *= tr(0.0, -0.3125, -0.4375);
        }
        (
            base,
            if wall { "sign_wall" } else { "sign_standing" },
            Vec3::new(0.6666667, -0.6666667, -0.6666667),
            0.015625 * 0.6666667,
            Vec3::new(0.0, 0.333_333_34, 0.046_666_667),
            10.0,
            90.0,
        )
    };
    let texture = if hanging {
        format!("minecraft:entity/signs/hanging/{wood}")
    } else {
        format!("minecraft:entity/signs/{wood}")
    };
    draws.push(model(
        &texture,
        model_name,
        base * Mat4::from_scale(model_scale),
    ));

    let center = ctx.eye.map(|e| {
        let d = [
            e[0] - (be.pos[0] as f64 + 0.5),
            e[1] - (be.pos[1] as f64 + 0.5),
            e[2] - (be.pos[2] as f64 + 0.5),
        ];
        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
    });
    let near = center.is_some_and(|d2| d2 < 16.0 * 16.0);
    for (key, front) in [("front_text", true), ("back_text", false)] {
        let Some(side) = nbt::get(&be.nbt, key).and_then(nbt::compound) else {
            continue;
        };
        let Some(Value::List(messages)) = side.get("messages") else {
            continue;
        };
        let color = nbt::get_str(side, "color").unwrap_or("black");
        let glowing = nbt::get_bool(side, "has_glowing_text");
        let text_color = dye_text_color(color);
        // AbstractSignRenderer.getDarkColor.
        let dark = if color == "black" && glowing {
            0xF0EBCC
        } else {
            scale_rgb(text_color, 0.4)
        };
        let (main, outline) = if glowing {
            (text_color, (color == "black" || near).then_some(dark))
        } else {
            (dark, None)
        };
        let mut m = base;
        if !front {
            m *= rot_y_deg(180.0);
        }
        m = m
            * Mat4::from_translation(offset)
            * Mat4::from_scale(Vec3::new(text_scale, -text_scale, text_scale));
        let half = (4.0 * line_h / 2.0_f32).floor();
        for (i, msg) in messages.iter().take(4).enumerate() {
            let spans: Vec<Span> = text::flatten(msg, lang);
            let (placed, width) = text::layout_line(font, &spans, max_w);
            if placed.iter().all(|p| p.c == ' ') {
                continue;
            }
            stats.text_lines += 1;
            // Java int division: -width / 2 truncates toward zero.
            let x0 = -((width as i32) / 2) as f32;
            let y0 = i as f32 * line_h - half;
            let before = draws.len();
            // drawInBatch8xOutline: the outline colour at the 8 neighbouring pixel
            // offsets, a hair behind, then the text itself.
            if let Some(oc) = outline {
                for (dx, dy) in [
                    (-1.0, -1.0),
                    (0.0, -1.0),
                    (1.0, -1.0),
                    (-1.0, 0.0),
                    (1.0, 0.0),
                    (-1.0, 1.0),
                    (0.0, 1.0),
                    (1.0, 1.0),
                ] {
                    glyph_draws(
                        font,
                        &placed,
                        m,
                        x0 + dx,
                        y0 + dy,
                        -0.1,
                        oc,
                        true,
                        true,
                        draws,
                    );
                }
            }
            glyph_draws(font, &placed, m, x0, y0, 0.0, main, glowing, false, draws);
            stats.glyphs += draws.len() - before;
        }
    }
    Some(if hanging { "hanging_sign" } else { "sign" })
}

/// Glyph quads of one laid-out line in text space (x right, y down, font pixels),
/// transformed by `m`. `forced` ignores the spans' own colours (outlines).
#[allow(clippy::too_many_arguments)]
fn glyph_draws(
    font: &Font,
    placed: &[text::Placed],
    m: Mat4,
    x: f32,
    y: f32,
    z: f32,
    color: u32,
    full_bright: bool,
    forced: bool,
    draws: &mut Vec<Draw>,
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
        let w = g.cell[2] * g.scale;
        let h = g.cell[3] * g.scale;
        let top = y + 7.0 - g.ascent;
        let copies: &[f32] = if p.bold { &[0.0, 1.0] } else { &[0.0] };
        for &bx in copies {
            let left = x + p.x + bx;
            draws.push(Draw::Glyph {
                texture: g.texture.clone(),
                tex_size: g.texture_size,
                uv: [
                    g.cell[0],
                    g.cell[1],
                    g.cell[0] + g.cell[2],
                    g.cell[1] + g.cell[3],
                ],
                p0: m.transform_point3(Vec3::new(left, top, z)),
                eu: m.transform_vector3(Vec3::new(w, 0.0, 0.0)),
                ev: m.transform_vector3(Vec3::new(0.0, h, 0.0)),
                rgb,
                full_bright,
            });
        }
    }
}

fn plan_bell(state: &BlockState, t: Mat4, draws: &mut Vec<Draw>) -> Option<&'static str> {
    if state.short_name() != "bell" {
        return None;
    }
    // BellRenderer draws the body at the block origin; the frame is the JSON model.
    draws.push(model("minecraft:entity/bell/bell_body", "bell", t));
    Some("bell")
}

fn plan_chest(state: &BlockState, t: Mat4, draws: &mut Vec<Draw>) -> Option<&'static str> {
    let name = state.short_name();
    let base = match name.strip_prefix("waxed_").unwrap_or(name) {
        "chest" => "normal",
        "trapped_chest" => "trapped",
        "ender_chest" => "ender",
        "copper_chest" => "copper",
        "exposed_copper_chest" => "copper_exposed",
        "weathered_copper_chest" => "copper_weathered",
        "oxidized_copper_chest" => "copper_oxidized",
        _ => return None,
    };
    let ty = if base == "ender" {
        "single"
    } else {
        state.prop("type").unwrap_or("single")
    };
    let (model_name, suffix) = match ty {
        "left" => ("chest_left", "_left"),
        "right" => ("chest_right", "_right"),
        _ => ("chest", ""),
    };
    let angle = facing_yrot(state.prop("facing").unwrap_or("north"));
    let m = t * tr(0.5, 0.5, 0.5) * rot_y_deg(-angle) * tr(-0.5, -0.5, -0.5);
    draws.push(model(
        &format!("minecraft:entity/chest/{base}{suffix}"),
        model_name,
        m,
    ));
    Some(if base == "ender" {
        "ender_chest"
    } else {
        "chest"
    })
}

/// BookModel.setupAnim for (animation 0, page flips, openness).
fn book_poses(flip1: f32, flip2: f32, open: f32) -> Vec<(&'static str, PoseEdit)> {
    let f = (0f32.sin() * 0.1 + 1.25) * open;
    let x = Some(f.sin());
    vec![
        (
            "left_lid",
            PoseEdit::rot(None, Some(std::f32::consts::PI + f), None),
        ),
        ("right_lid", PoseEdit::rot(None, Some(-f), None)),
        (
            "left_pages",
            PoseEdit {
                pivot_x: x,
                rotation: [None, Some(f), None],
            },
        ),
        (
            "right_pages",
            PoseEdit {
                pivot_x: x,
                rotation: [None, Some(-f), None],
            },
        ),
        (
            "flip_page1",
            PoseEdit {
                pivot_x: x,
                rotation: [None, Some(f - f * 2.0 * flip1), None],
            },
        ),
        (
            "flip_page2",
            PoseEdit {
                pivot_x: x,
                rotation: [None, Some(f - f * 2.0 * flip2), None],
            },
        ),
    ]
}

const BOOK_TEXTURE: &str = "minecraft:entity/enchanting_table_book";

fn plan_lectern(state: &BlockState, t: Mat4, draws: &mut Vec<Draw>) -> Option<&'static str> {
    if state.short_name() != "lectern" || state.prop("has_book") != Some("true") {
        return None;
    }
    // LecternRenderer: yRot = facing.getClockWise().toYRot(); book open at 1.2.
    let cw = match state.prop("facing").unwrap_or("north") {
        "north" => "east",
        "east" => "south",
        "south" => "west",
        _ => "north",
    };
    let m = t
        * tr(0.5, 1.0625, 0.5)
        * rot_y_deg(-facing_yrot(cw))
        * Mat4::from_rotation_z(67.5f32.to_radians())
        * tr(0.0, -0.125, 0.0);
    draws.push(Draw::Model {
        model: "book",
        texture: BOOK_TEXTURE.into(),
        m,
        poses: book_poses(0.1, 0.9, 1.2),
        only: Vec::new(),
    });
    Some("lectern_book")
}

fn plan_enchanting_table(
    state: &BlockState,
    t: Mat4,
    draws: &mut Vec<Draw>,
) -> Option<&'static str> {
    if state.short_name() != "enchanting_table" {
        return None;
    }
    // EnchantTableRenderer at rest (time 0, no reader): the book floats closed,
    // tilted 80 degrees; page flips at frac(0.25)*1.6-0.3 and frac(0.75)*1.6-0.3.
    let m = t * tr(0.5, 0.75 + 0.1, 0.5) * Mat4::from_rotation_z(80f32.to_radians());
    draws.push(Draw::Model {
        model: "book",
        texture: BOOK_TEXTURE.into(),
        m,
        poses: book_poses(0.1, 0.9, 0.0),
        only: Vec::new(),
    });
    Some("enchanting_table_book")
}

/// The texture asset of a banner pattern layer (`patterns[].pattern`): a registry id
/// whose asset is `entity/banner/<path>`, or an inline `{asset_id}`.
fn banner_pattern_texture(v: &Value) -> Option<String> {
    let asset = match v {
        Value::String(s) => s.clone(),
        Value::Compound(c) => nbt::get_str(c, "asset_id")?.to_string(),
        _ => return None,
    };
    let (ns, path) = split_loc(&asset);
    Some(format!("{ns}:entity/banner/{path}"))
}

fn plan_banner(
    ctx: &PlanContext,
    be: &BlockEntity,
    state: &BlockState,
    t: Mat4,
    generated: &mut Vec<(String, u32, u32, Vec<u32>)>,
    draws: &mut Vec<Draw>,
) -> Option<&'static str> {
    let name = state.short_name();
    let (color, standing) = if let Some(c) = name.strip_suffix("_wall_banner") {
        (c, false)
    } else {
        (name.strip_suffix("_banner")?, true)
    };
    let base_color = dye_diffuse_color(color)?;
    let angle = if standing {
        -rotation_degrees(state)
    } else {
        -facing_yrot(state.prop("facing").unwrap_or("north"))
    };
    let m = t
        * tr(0.5, 0.0, 0.5)
        * rot_y_deg(angle)
        * Mat4::from_scale(Vec3::new(0.6666667, -0.6666667, -0.6666667));
    draws.push(model(
        "minecraft:entity/banner_base",
        if standing {
            "banner_standing"
        } else {
            "banner_wall"
        },
        m,
    ));
    // The flag: banner_base, then entity/banner/base in the base colour, then up to
    // 16 pattern layers, each alpha-blended in its colour (BannerRenderer
    // .submitPatterns). Composited once into one texture per distinct banner.
    let mut layers: Vec<(String, u32)> = vec![("minecraft:entity/banner/base".into(), base_color)];
    if let Some(Value::List(pats)) = be.nbt.get("patterns") {
        for p in pats.iter().filter_map(nbt::compound).take(16) {
            let Some(tex) = p.get("pattern").and_then(banner_pattern_texture) else {
                continue;
            };
            let c = nbt::get_str(p, "color")
                .and_then(dye_diffuse_color)
                .unwrap_or(0xF9FFFE);
            layers.push((tex, c));
        }
    }
    let key = layers
        .iter()
        .map(|(t, c)| format!("{}={c:06x}", t.rsplit('/').next().unwrap_or(t)))
        .collect::<Vec<_>>()
        .join(",");
    let loc = format!("lumenwright:banner/{key}");
    if !generated.iter().any(|g| g.0 == loc) {
        let (w, h, mut px) = load_texture(ctx.store, "minecraft:entity/banner_base").unwrap_or((
            64,
            64,
            vec![0xFF80_8080; 64 * 64],
        ));
        for (tex, c) in &layers {
            let Some((lw, lh, lpx)) = load_texture(ctx.store, tex) else {
                log::debug!("banner pattern {tex} not found");
                continue;
            };
            if (lw, lh) != (w, h) {
                continue;
            }
            for (d, s) in px.iter_mut().zip(&lpx) {
                *d = blend(*d, *s, *c);
            }
        }
        generated.push((loc.clone(), w, h, px));
    }
    let phase = (be.pos[0] * 7 + be.pos[1] * 9 + be.pos[2] * 13).rem_euclid(100) as f32 / 100.0;
    let xrot = (-0.0125 + 0.01 * (std::f32::consts::TAU * phase).cos()) * std::f32::consts::PI;
    draws.push(Draw::Model {
        model: if standing {
            "banner_flag_standing"
        } else {
            "banner_flag_wall"
        },
        texture: loc,
        m,
        poses: vec![("flag", PoseEdit::rot(Some(xrot), None, None))],
        only: Vec::new(),
    });
    Some("banner")
}

/// Alpha-blends a tinted layer texel over `dst` (RGBA8, red low), keeping dst alpha.
fn blend(dst: u32, src: u32, tint: u32) -> u32 {
    let a = (src >> 24) as f32 / 255.0;
    if a <= 0.0 {
        return dst;
    }
    let ch = |v: u32, s: u32| ((v >> s) & 0xFF) as f32;
    let mut out = dst & 0xFF00_0000;
    for (s, ts) in [(0, 16), (8, 8), (16, 0)] {
        let sc = ch(src, s) * ch(tint, ts) / 255.0;
        let v = ch(dst, s) * (1.0 - a) + sc * a;
        out |= (v.round().clamp(0.0, 255.0) as u32) << s;
    }
    out
}

fn plan_decorated_pot(
    be: &BlockEntity,
    state: &BlockState,
    t: Mat4,
    draws: &mut Vec<Draw>,
) -> Option<&'static str> {
    if state.short_name() != "decorated_pot" {
        return None;
    }
    let dir = facing_yrot(state.prop("facing").unwrap_or("north"));
    let m = t * tr(0.5, 0.0, 0.5) * rot_y_deg(180.0 - dir) * tr(-0.5, 0.0, -0.5);
    draws.push(model(
        "minecraft:entity/decorated_pot/decorated_pot_base",
        "decorated_pot_base",
        m,
    ));
    // PotDecorations: sherds [back, left, right, front]; a pottery sherd item names
    // the pattern `<name>_pottery_pattern`, anything else (brick) the plain side.
    let sherds: Vec<String> = match be.nbt.get("sherds") {
        Some(Value::List(l)) => l
            .iter()
            .map(|v| nbt::as_str(v).unwrap_or("minecraft:brick").to_string())
            .collect(),
        _ => Vec::new(),
    };
    for (i, side) in ["back", "left", "right", "front"].into_iter().enumerate() {
        let tex = sherds
            .get(i)
            .and_then(|s| {
                let (ns, path) = split_loc(s);
                let name = path.strip_suffix("_pottery_sherd")?;
                Some(format!("{ns}:entity/decorated_pot/{name}_pottery_pattern"))
            })
            .unwrap_or_else(|| "minecraft:entity/decorated_pot/decorated_pot_side".into());
        draws.push(Draw::Model {
            model: "decorated_pot_sides",
            texture: tex,
            m,
            poses: Vec::new(),
            only: vec![side],
        });
    }
    Some("decorated_pot")
}

/// The 18 default skins in `DefaultPlayerSkin` order.
const DEFAULT_SKINS: [&str; 18] = [
    "slim/alex",
    "slim/ari",
    "slim/efe",
    "slim/kai",
    "slim/makena",
    "slim/noor",
    "slim/steve",
    "slim/sunny",
    "slim/zuri",
    "wide/alex",
    "wide/ari",
    "wide/efe",
    "wide/kai",
    "wide/makena",
    "wide/noor",
    "wide/steve",
    "wide/sunny",
    "wide/zuri",
];

/// A player head's skin from the save alone: the profile's `texture` asset when a
/// resource pack provides it, else the default skin for the profile's UUID
/// (`DefaultPlayerSkin.get`), else Steve. Skins are never fetched.
fn player_head_texture(ctx: &PlanContext, be: &BlockEntity) -> String {
    if let Some(tex) = nbt::get_str(&be.nbt, "profile.texture") {
        let (ns, path) = split_loc(tex);
        for cand in [format!("{ns}:{path}"), format!("{ns}:entity/{path}")] {
            if ctx.store.resolve("textures", &cand, "png").is_some() {
                return cand;
            }
        }
        log::debug!("player head skin {tex} not found; using a default skin");
    }
    if let Some(id) = nbt::get(&be.nbt, "profile.id").and_then(nbt::as_f64_list)
        && id.len() == 4
    {
        // UUID.hashCode() of the int-array form is the XOR of its four ints.
        let h = id.iter().fold(0i32, |a, v| a ^ (*v as i64 as i32));
        let i = h.rem_euclid(DEFAULT_SKINS.len() as i32) as usize;
        return format!("minecraft:entity/player/{}", DEFAULT_SKINS[i]);
    }
    "minecraft:entity/player/wide/steve".into()
}

fn plan_skull(
    ctx: &PlanContext,
    be: &BlockEntity,
    state: &BlockState,
    t: Mat4,
    draws: &mut Vec<Draw>,
) -> Option<&'static str> {
    let name = state.short_name();
    let (ty, wall) = if let Some(n) = name.strip_suffix("_wall_skull") {
        (n, true)
    } else if let Some(n) = name.strip_suffix("_wall_head") {
        (n, true)
    } else if let Some(n) = name.strip_suffix("_skull") {
        (n, false)
    } else {
        (name.strip_suffix("_head")?, false)
    };
    let (model_name, texture) = match ty {
        "skeleton" => (
            "skull_mob",
            "minecraft:entity/skeleton/skeleton".to_string(),
        ),
        "wither_skeleton" => (
            "skull_mob",
            "minecraft:entity/skeleton/wither_skeleton".to_string(),
        ),
        "creeper" => ("skull_mob", "minecraft:entity/creeper/creeper".to_string()),
        "zombie" => (
            "skull_humanoid",
            "minecraft:entity/zombie/zombie".to_string(),
        ),
        "player" => ("skull_humanoid", player_head_texture(ctx, be)),
        "dragon" => (
            "skull_dragon",
            "minecraft:entity/enderdragon/dragon".to_string(),
        ),
        "piglin" => ("skull_piglin", "minecraft:entity/piglin/piglin".to_string()),
        _ => return None,
    };
    // SkullBlockRenderer.submitSkull.
    let (base, deg) = if wall {
        let facing = state.prop("facing").unwrap_or("north");
        let (sx, sz) = facing_step(facing);
        let opposite = match facing {
            "north" => "south",
            "south" => "north",
            "west" => "east",
            _ => "west",
        };
        (
            t * tr(0.5 - sx * 0.25, 0.25, 0.5 - sz * 0.25),
            facing_yrot(opposite),
        )
    } else {
        (t * tr(0.5, 0.0, 0.5), rotation_degrees(state))
    };
    let m = base * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
    let mut poses = vec![(
        "head",
        PoseEdit::rot(Some(0.0), Some(deg.to_radians()), None),
    )];
    match ty {
        // DragonHeadModel / PiglinHeadModel.setupAnim at animation 0.
        "dragon" => poses.push(("jaw", PoseEdit::rot(Some(0.2), None, None))),
        "piglin" => {
            poses.push((
                "left_ear",
                PoseEdit::rot(None, None, Some(-(1.0 + 2.5) * 0.2)),
            ));
            poses.push((
                "right_ear",
                PoseEdit::rot(None, None, Some((1.0 + 2.5) * 0.2)),
            ));
        }
        _ => {}
    }
    draws.push(Draw::Model {
        model: model_name,
        texture,
        m,
        poses,
        only: Vec::new(),
    });
    Some(if ty == "player" {
        "player_head"
    } else {
        "skull"
    })
}

/// Emits the planned geometry as entity quads (after the atlas is packed).
pub fn build_quads(scene: &mut Scene, models: &BlockEntityModels, plan: &BlockEntityPlan) {
    let mut quads = Vec::new();
    let mut lights = Vec::new();
    for p in &plan.planned {
        for d in &p.draws {
            let before = quads.len();
            let mut light = EntityLight::shaded(p.rel);
            match d {
                Draw::Model {
                    model,
                    texture,
                    m,
                    poses,
                    only,
                } => {
                    let Some(def) = models.models.get(*model) else {
                        log::warn!("block entity model {model} missing from the bundle");
                        continue;
                    };
                    let sprite = scene.atlas.sprite(texture);
                    let pose = |name: &str, pp: &mut PartPose| {
                        for (n, e) in poses {
                            if *n == name {
                                if let Some(x) = e.pivot_x {
                                    pp.pivot[0] = x;
                                }
                                for i in 0..3 {
                                    if let Some(r) = e.rotation[i] {
                                        pp.rotation[i] = r;
                                    }
                                }
                            }
                        }
                    };
                    for part in &def.parts {
                        if !only.is_empty() && !only.contains(&part.name.as_str()) {
                            continue;
                        }
                        part_quads(part, m, &sprite, def.texture_size, &pose, &mut quads);
                    }
                }
                Draw::Glyph {
                    texture,
                    tex_size,
                    uv,
                    p0,
                    eu,
                    ev,
                    rgb,
                    full_bright,
                } => {
                    let sprite = scene.atlas.sprite(texture);
                    let a = sprite.to_atlas(uv[0], uv[1], *tex_size);
                    let b = sprite.to_atlas(uv[2], uv[1], *tex_size);
                    let c = sprite.to_atlas(uv[0], uv[3], *tex_size);
                    quads.push(Quad {
                        p0: *p0,
                        eu: *eu,
                        ev: *ev,
                        uv0: a,
                        uvs: [b[0] - a[0], b[1] - a[1]],
                        uvt: [c[0] - a[0], c[1] - a[1]],
                        tint: [(rgb >> 16) as u8, (rgb >> 8) as u8, *rgb as u8, 255],
                        alpha: Alpha::Cutout,
                        water: false,
                        shade: false,
                        // HQ: glowing text gives off about its own albedo under the
                        // default emitter strength, so it reads as lit, not as a lamp.
                        emission: if *full_bright { 0.2 } else { 0.0 },
                        cullface: None,
                        ao: false,
                    });
                    light = EntityLight {
                        pos: p.rel,
                        shade: false,
                        full_bright: *full_bright,
                    };
                }
            }
            lights.extend(std::iter::repeat_n(light, quads.len() - before));
        }
    }
    log::debug!(
        "block entity quads: {} from {} block entities",
        quads.len(),
        plan.planned.len()
    );
    scene.entity_quads.extend(quads);
    scene.entity_quad_light.extend(lights);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_models_parse_and_cite_their_source() {
        let m = BlockEntityModels::load_builtin();
        assert_eq!(m.format, 1);
        for k in [
            "chest",
            "chest_left",
            "chest_right",
            "bell",
            "sign_standing",
            "sign_wall",
            "hanging_sign_wall",
            "hanging_sign_ceiling",
            "hanging_sign_ceiling_middle",
            "banner_standing",
            "banner_wall",
            "banner_flag_standing",
            "banner_flag_wall",
            "book",
            "skull_mob",
            "skull_humanoid",
            "skull_dragon",
            "skull_piglin",
            "decorated_pot_base",
            "decorated_pot_sides",
        ] {
            let d = m.models.get(k).unwrap_or_else(|| panic!("{k}"));
            assert!(
                d.source
                    .as_deref()
                    .is_some_and(|s| s.starts_with("net.minecraft.")),
                "{k} cites no vanilla class"
            );
        }
    }

    #[test]
    fn dye_colours_match_dyecolor() {
        assert_eq!(dye_text_color("black"), 0);
        assert_eq!(scale_rgb(0xFFFFFF, 0.4), 0x666666);
        assert_eq!(dye_diffuse_color("red"), Some(0xB02E26));
        assert_eq!(dye_diffuse_color("nope"), None);
    }

    #[test]
    fn banner_layers_blend_in_their_colour() {
        // An opaque white layer tinted red replaces the colour; a clear one keeps it.
        let dst = 0xFF00_FF00;
        assert_eq!(blend(dst, 0xFFFF_FFFF, 0xFF0000), 0xFF00_00FF);
        assert_eq!(blend(dst, 0x00FF_FFFF, 0xFF0000), dst);
    }

    /// Corners of a planned model draw, posed, with a dummy sprite.
    fn draw_corners(d: &Draw, part: &str) -> Vec<Vec3> {
        let Draw::Model {
            model, m, poses, ..
        } = d
        else {
            panic!("not a model draw");
        };
        let models = BlockEntityModels::load_builtin();
        let def = &models.models[*model];
        let sprite = crate::assets::atlas::Sprite {
            x: 0,
            y: 0,
            w: 64,
            h: 64,
            transparent: false,
            translucent: false,
        };
        let pose = |name: &str, pp: &mut PartPose| {
            for (n, e) in poses {
                if *n == name {
                    for i in 0..3 {
                        if let Some(r) = e.rotation[i] {
                            pp.rotation[i] = r;
                        }
                    }
                }
            }
        };
        let mut out = Vec::new();
        for p in def.parts.iter().filter(|p| p.name == part) {
            part_quads(p, m, &sprite, def.texture_size, &pose, &mut out);
        }
        out.iter().flat_map(|q| q.corners()).collect()
    }

    #[test]
    fn chest_lock_faces_the_facing_direction() {
        for (facing, axis, sign) in [("south", 2, 1.0), ("north", 2, -1.0), ("east", 0, 1.0)] {
            let st = BlockState::new("minecraft:chest")
                .with("facing", facing)
                .with("type", "single");
            let mut draws = Vec::new();
            plan_chest(&st, Mat4::IDENTITY, &mut draws).unwrap();
            let c = draw_corners(&draws[0], "lock");
            let mean = c.iter().map(|v| v[axis]).sum::<f32>() / c.len() as f32;
            assert!((mean - 0.5) * sign > 0.4, "{facing}: lock at {mean}");
        }
    }

    #[test]
    fn skull_rotation_8_faces_south() {
        // The face is the head's north cube face; rotation 8 turns it to +Z, and a
        // wall skull facing south looks south too.
        for (name, props) in [
            ("minecraft:zombie_head", ("rotation", "8")),
            ("minecraft:zombie_wall_head", ("facing", "south")),
        ] {
            let st = BlockState::new(name).with(props.0, props.1);
            let be = BlockEntity {
                id: "minecraft:skull".into(),
                pos: [0, 0, 0],
                nbt: Compound::new(),
            };
            let store = ResourceStore::new();
            let registry = Registry::new();
            let volume = BlockVolume {
                min_chunk: [0, 0],
                chunks: [1, 1],
                min_section: 0,
                sections_y: 1,
                sections: vec![None],
            };
            let ctx = PlanContext {
                store: &store,
                registry: &registry,
                volume: &volume,
                origin: [0, 0, 0],
                eye: None,
            };
            let mut draws = Vec::new();
            plan_skull(&ctx, &be, &st, Mat4::IDENTITY, &mut draws).unwrap();
            let Draw::Model { m, poses, .. } = &draws[0] else {
                panic!()
            };
            let yrot = poses[0].1.rotation[1].unwrap();
            // Model-space -Z (the face), through the head's yaw and the block transform.
            let face = m.transform_vector3(Mat4::from_rotation_y(yrot).transform_vector3(-Vec3::Z));
            assert!(face.z > 0.99, "{name}: face points {face}");
        }
    }

    #[test]
    fn sign_woods() {
        assert_eq!(sign_wood("dark_oak_wall_hanging_sign"), Some("dark_oak"));
        assert_eq!(sign_wood("oak_sign"), Some("oak"));
        assert_eq!(sign_wood("stone"), None);
    }
}
