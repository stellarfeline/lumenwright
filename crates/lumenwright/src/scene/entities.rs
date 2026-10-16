//! Entity geometry: data-driven cuboid mob models (`assets/entity_models.json`),
//! block displays, paintings, item frames, the items entities carry (dropped
//! items, item displays, frame contents, held items and worn armour) and hitbox
//! fallbacks.

use std::collections::{BTreeMap, HashMap};

use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;

use super::items::{ItemBakery, Prepared, bounds, rendered_amount};
use super::{Alpha, EntityLight, Quad, Scene};
use crate::assets::atlas::{AtlasBuilder, WHITE_TEXTURE};
use crate::assets::blockstate::pos_hash;
use crate::assets::items::{DisplayContext, ItemStack};
use crate::assets::source::{ResourceStore, norm_loc};
use crate::world::nbt::{self, Compound};
use crate::world::{BlockState, Entity};

const BUILTIN_MODELS: &str = include_str!("../../assets/entity_models.json");

#[derive(Deserialize, Clone, Debug)]
pub struct CubeDef {
    pub uv: [f32; 2],
    pub origin: [f32; 3],
    pub size: [f32; 3],
    #[serde(default)]
    pub inflate: f32,
    #[serde(default)]
    pub mirror: bool,
    /// Faces to emit (vanilla `addBox(..., Set<Direction>)`), by direction name in
    /// model space; all six when absent.
    #[serde(default)]
    pub faces: Option<Vec<String>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct PartDef {
    pub name: String,
    #[serde(default)]
    pub pivot: [f32; 3],
    #[serde(default)]
    pub rotation: [f32; 3],
    /// Part scale (vanilla `PartPose.scaled`), applied after the rotation.
    #[serde(default = "unit_scale")]
    pub scale: [f32; 3],
    #[serde(default)]
    pub cubes: Vec<CubeDef>,
    #[serde(default)]
    pub children: Vec<PartDef>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TextureVariants {
    pub field: String,
    pub pattern: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ModelDef {
    /// Vanilla class and method the geometry is transcribed from.
    #[serde(default)]
    pub source: Option<String>,
    /// Default texture; block-entity models pick theirs in code and leave it empty.
    #[serde(default)]
    pub texture: String,
    pub texture_variants: Option<TextureVariants>,
    pub texture_size: [f32; 2],
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default = "default_hitbox")]
    pub hitbox: [f32; 2],
    pub parts: Vec<PartDef>,
}

fn one() -> f32 {
    1.0
}
fn unit_scale() -> [f32; 3] {
    [1.0; 3]
}
fn default_hitbox() -> [f32; 2] {
    [0.6, 1.8]
}

#[derive(Deserialize)]
pub struct EntityModels {
    pub format: u32,
    pub models: HashMap<String, ModelDef>,
}

impl EntityModels {
    pub fn load_builtin() -> EntityModels {
        serde_json::from_str(BUILTIN_MODELS).expect("bundled entity_models.json is valid")
    }
}

/// What the entity pass drew, by kind (entity ids without the namespace).
#[derive(Default, Debug, Clone)]
pub struct EntityStats {
    /// Entities with something of them drawn: a body, or for a dropped item,
    /// an item display or an invisible holder, the item.
    pub rendered: BTreeMap<String, usize>,
    pub fallback_boxes: BTreeMap<String, usize>,
    /// Entities nothing of which is drawn: a kind without a model under
    /// `UnknownEntities::Skip`, or a dropped item or item display whose item
    /// cannot be drawn.
    pub skipped: BTreeMap<String, usize>,
    /// Item stacks drawn (dropped, displayed, framed, held or worn), by item id.
    pub items_rendered: BTreeMap<String, usize>,
    /// Item stacks that could not be drawn, by item id (the reason is logged):
    /// a special model other than the shield, a definition the resources lack.
    pub items_skipped: BTreeMap<String, usize>,
}

/// Entities that have no visible body.
fn is_invisible_kind(id: &str) -> bool {
    matches!(
        id,
        "marker" | "interaction" | "area_effect_cloud" | "lightning_bolt"
    )
}

/// Hitbox sizes for common entities without a model, so fallback boxes look sane.
fn fallback_hitbox(id: &str) -> [f32; 2] {
    match id {
        "squid" | "glow_squid" => [0.8, 0.8],
        "bat" => [0.5, 0.9],
        "item" | "experience_orb" => [0.25, 0.25],
        "boat" | "chest_boat" => [1.375, 0.5625],
        "minecart" | "chest_minecart" | "hopper_minecart" | "tnt_minecart" | "furnace_minecart" => {
            [0.98, 0.7]
        }
        "tnt" | "falling_block" => [0.98, 0.98],
        "dolphin" => [0.9, 0.6],
        "turtle" => [1.2, 0.4],
        "axolotl" => [0.75, 0.42],
        "cod" | "salmon" | "tropical_fish" | "pufferfish" => [0.5, 0.4],
        "allay" | "vex" => [0.35, 0.6],
        "parrot" => [0.5, 0.9],
        "warden" => [0.9, 2.9],
        "camel" => [1.7, 2.375],
        "sniffer" => [1.9, 1.75],
        "armadillo" => [0.7, 0.65],
        "breeze" => [0.6, 1.77],
        "creaking" => [0.9, 2.7],
        "happy_ghast" => [4.0, 4.0],
        _ => [0.6, 1.8],
    }
}

fn texture_exists(store: &ResourceStore, loc: &str) -> bool {
    store.resolve("textures", loc, "png").is_some()
}

/// The texture an entity should use (variant, mannequin skin...), as `ns:path`.
fn entity_texture(def: &ModelDef, e: &Entity, store: &ResourceStore) -> String {
    let default = norm_loc(&def.texture);
    if let Some(v) = &def.texture_variants
        && let Some(val) = e.str_field(&v.field)
    {
        let name = val.rsplit(':').next().unwrap_or(val);
        let loc = norm_loc(&v.pattern.replace("{name}", name));
        if texture_exists(store, &loc) {
            return loc;
        }
    }
    if (e.short_id() == "mannequin" || e.short_id() == "player")
        && let Some(tex) = nbt::get_str(&e.nbt, "profile.texture")
    {
        let (ns, path) = crate::assets::source::split_loc(tex);
        for cand in [format!("{ns}:{path}"), format!("{ns}:entity/{path}")] {
            if texture_exists(store, &cand) {
                return cand;
            }
        }
        log::debug!("mannequin skin {tex} not found; using the default skin");
    }
    default
}

fn painting_variant(e: &Entity) -> Option<String> {
    let v = e.str_field("variant").or_else(|| e.str_field("Motive"))?;
    let (ns, path) = crate::assets::source::split_loc(v);
    Some(format!("{ns}:painting/{path}"))
}

/// Block states referenced by block displays (interned before baking).
pub fn display_states(entities: &[Entity]) -> Vec<BlockState> {
    entities
        .iter()
        .filter_map(|e| {
            if e.short_id() == "block_display" {
                block_state_from_nbt(nbt::get(&e.nbt, "block_state").and_then(nbt::compound)?)
            } else {
                item_frame_state(e)
            }
        })
        .collect()
}

/// Item frames render through their vanilla block model.
fn item_frame_state(e: &Entity) -> Option<BlockState> {
    match e.short_id() {
        "item_frame" | "glow_item_frame" => Some(BlockState::new(&e.id).with("map", "false")),
        _ => None,
    }
}

/// Moves a scene-space quad by `m`. Mirroring transforms flip the winding;
/// that is undone so normals stay outward.
pub(crate) fn transform_quad(q: &mut Quad, m: &Mat4) {
    let p0 = m.transform_point3(q.p0);
    q.eu = m.transform_vector3(q.eu);
    q.ev = m.transform_vector3(q.ev);
    q.p0 = p0;
    q.cullface = None;
    if m.determinant() < 0.0 {
        q.p0 += q.eu;
        q.eu = -q.eu;
        q.uv0 = [q.uv0[0] + q.uvs[0], q.uv0[1] + q.uvs[1]];
        q.uvs = [-q.uvs[0], -q.uvs[1]];
    }
}

/// Emits the baked model of a block state, transformed by `m` (block-local 0..1 in).
fn push_block_model(
    scene: &mut Scene,
    state: &BlockState,
    m: &Mat4,
    rel: Vec3,
    scratch: &mut Vec<u32>,
    quads: &mut Vec<Quad>,
) {
    let sid = scene.registry.intern(state);
    let info = scene.blocks.get(sid as usize).cloned().unwrap_or_default();
    let p = rel.floor();
    info.models
        .choose(p.x as i32, p.y as i32, p.z as i32, scratch);
    for &mid in scratch.iter() {
        let model = &scene.baked[mid as usize];
        for bq in &model.quads {
            let mut q = scene.resolve_quad(bq, 0xFFFFFF, false, info.translucent, info.emission);
            transform_quad(&mut q, m);
            quads.push(q);
        }
    }
}

fn block_state_from_nbt(c: &Compound) -> Option<BlockState> {
    let name = nbt::get_str(c, "Name")?;
    let mut s = BlockState::new(&norm_loc(name));
    if let Some(props) = nbt::get(c, "Properties").and_then(nbt::compound) {
        for (k, v) in props {
            if let Some(v) = nbt::as_str(v) {
                s = s.with(k, v);
            }
        }
    }
    Some(s)
}

pub fn register_textures(models: &EntityModels, entities: &[Entity], ab: &mut AtlasBuilder) {
    ab.add(WHITE_TEXTURE);
    ab.add("minecraft:painting/back");
    // Textures are resolved again (with existence checks) when building quads;
    // registering every candidate here is harmless (missing ones fall back).
    for e in entities {
        if let Some(def) = models.models.get(&e.id) {
            ab.add(&norm_loc(&def.texture));
            if let Some(v) = &def.texture_variants
                && let Some(val) = e.str_field(&v.field)
            {
                let name = val.rsplit(':').next().unwrap_or(val);
                ab.add(&norm_loc(&v.pattern.replace("{name}", name)));
            }
            if let Some(tex) = nbt::get_str(&e.nbt, "profile.texture") {
                let (ns, path) = crate::assets::source::split_loc(tex);
                ab.add(&format!("{ns}:{path}"));
                ab.add(&format!("{ns}:entity/{path}"));
            }
        }
        if e.short_id() == "painting"
            && let Some(v) = painting_variant(e)
        {
            ab.add(&v);
        }
    }
}

/// Half-gap, in model pixels, between the two faces of a zero-thickness cube.
const FLAT_EPSILON: f32 = 0.01;

/// Emits the 6 faces of a box-UV cube (vanilla `ModelPart.Cube`) transformed by `m`.
pub(crate) fn cube_quads(
    c: &CubeDef,
    m: &Mat4,
    sprite: &crate::assets::atlas::Sprite,
    tex_size: [f32; 2],
    out: &mut Vec<Quad>,
) {
    // A zero-thickness box (a flat plane) has two coplanar faces; they are pushed
    // apart by a hair so a ray always meets the outward one first.
    let g = |i: usize| {
        if c.size[i] == 0.0 {
            c.inflate + FLAT_EPSILON
        } else {
            c.inflate
        }
    };
    let (mut x0, y0, z0) = (c.origin[0] - g(0), c.origin[1] - g(1), c.origin[2] - g(2));
    let (mut x1, y1, z1) = (
        c.origin[0] + c.size[0] + g(0),
        c.origin[1] + c.size[1] + g(1),
        c.origin[2] + c.size[2] + g(2),
    );
    if c.mirror {
        std::mem::swap(&mut x0, &mut x1);
    }
    let v = [
        Vec3::new(x0, y0, z0),
        Vec3::new(x1, y0, z0),
        Vec3::new(x1, y1, z0),
        Vec3::new(x0, y1, z0),
        Vec3::new(x0, y0, z1),
        Vec3::new(x1, y0, z1),
        Vec3::new(x1, y1, z1),
        Vec3::new(x0, y1, z1),
    ];
    let [w, h, d] = c.size;
    let (u, vv) = (c.uv[0], c.uv[1]);
    let (f7, f8, f9, f10, f11, f12) = (
        u,
        u + d,
        u + d + w,
        u + d + w + w,
        u + d + w + d,
        u + d + w + d + w,
    );
    let (f13, f14, f15) = (vv, vv + d, vv + d + h);
    // (direction, vertices, u1, v1, u2, v2) per vanilla polygon.
    type Face<'a> = (&'a str, [usize; 4], f32, f32, f32, f32);
    let faces: [Face; 6] = [
        ("down", [5, 4, 0, 1], f8, f13, f9, f14),
        ("up", [2, 3, 7, 6], f9, f14, f10, f13),
        ("west", [0, 4, 7, 3], f7, f14, f8, f15),
        ("north", [1, 0, 3, 2], f8, f14, f9, f15),
        ("east", [5, 1, 2, 6], f9, f14, f11, f15),
        ("south", [4, 5, 6, 7], f11, f14, f12, f15),
    ];
    let center = m.transform_point3((v[0] + v[6]) * 0.5);
    for (dir, idx, u1, v1, u2, v2) in faces {
        if let Some(only) = &c.faces
            && !only.iter().any(|f| f == dir)
        {
            continue;
        }
        let p = idx.map(|i| m.transform_point3(v[i]));
        // Vanilla polygon: vertex0 = (u2, v1), vertex1 = (u1, v1), vertex2 = (u1, v2).
        let p0 = p[1];
        let eu = p[0] - p[1];
        let ev = p[2] - p[1];
        if eu.cross(ev).length_squared() < 1e-14 {
            continue;
        }
        let a = sprite.to_atlas(u1, v1, tex_size);
        let b = sprite.to_atlas(u2, v1, tex_size);
        let cc = sprite.to_atlas(u1, v2, tex_size);
        let mut q = Quad {
            p0,
            eu,
            ev,
            uv0: a,
            uvs: [b[0] - a[0], b[1] - a[1]],
            uvt: [cc[0] - a[0], cc[1] - a[1]],
            tint: [255; 4],
            alpha: Alpha::Cutout,
            water: false,
            shade: true,
            emission: 0.0,
            cullface: None,
            ao: false,
        };
        orient_outward(&mut q, center);
        out.push(q);
    }
}

/// Flips a quad so its normal points away from `center` (keeps the texture mapping).
fn orient_outward(q: &mut Quad, center: Vec3) {
    let c = q.p0 + (q.eu + q.ev) * 0.5;
    if q.normal().dot(c - center) < 0.0 {
        q.p0 += q.eu;
        q.eu = -q.eu;
        q.uv0 = [q.uv0[0] + q.uvs[0], q.uv0[1] + q.uvs[1]];
        q.uvs = [-q.uvs[0], -q.uvs[1]];
    }
}

/// A part's animated pose: what vanilla's `setupAnim` writes into a `ModelPart`.
#[derive(Clone, Copy, Debug)]
pub struct PartPose {
    pub pivot: [f32; 3],
    pub rotation: [f32; 3],
    pub visible: bool,
}

/// Sets a part's pose by name; the identity leaves the pose of the model file.
pub type PoseFn<'a> = &'a dyn Fn(&str, &mut PartPose);

pub fn rest_pose(_: &str, _: &mut PartPose) {}

/// A posed part's transform (vanilla `ModelPart.translateAndRotate`):
/// translate(pivot / 16), rotate ZYX, scale.
pub(crate) fn part_matrix(p: &PartPose, scale: [f32; 3]) -> Mat4 {
    let rot = p.rotation;
    Mat4::from_translation(Vec3::from(p.pivot) / 16.0)
        * Mat4::from_quat(Quat::from_euler(
            glam::EulerRot::ZYX,
            rot[2],
            rot[1],
            rot[0],
        ))
        * Mat4::from_scale(Vec3::from(scale))
}

pub(crate) fn part_quads(
    part: &PartDef,
    parent: &Mat4,
    sprite: &crate::assets::atlas::Sprite,
    tex_size: [f32; 2],
    pose: PoseFn,
    out: &mut Vec<Quad>,
) {
    let mut p = PartPose {
        pivot: part.pivot,
        rotation: part.rotation,
        visible: true,
    };
    pose(&part.name, &mut p);
    if !p.visible {
        return;
    }
    let m = *parent * part_matrix(&p, part.scale);
    let to_px = m * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
    for c in &part.cubes {
        cube_quads(c, &to_px, sprite, tex_size, out);
    }
    for child in &part.children {
        part_quads(child, &m, sprite, tex_size, pose, out);
    }
}

/// Unit cube quads (0..1) with a single sprite, transformed by `m`.
fn box_quads(m: &Mat4, sprite: &crate::assets::atlas::Sprite, tint: [u8; 4], out: &mut Vec<Quad>) {
    let model = crate::assets::model::cube_all(WHITE_TEXTURE);
    let center = m.transform_point3(Vec3::splat(0.5));
    for bq in &model.quads {
        let p0 = m.transform_point3(bq.p0);
        let eu = m.transform_vector3(bq.eu);
        let ev = m.transform_vector3(bq.ev);
        let a = sprite.to_atlas(0.5, 0.5, [16.0, 16.0]);
        let mut q = Quad {
            p0,
            eu,
            ev,
            uv0: a,
            uvs: [sprite.w as f32 * 0.8, 0.0],
            uvt: [0.0, sprite.h as f32 * 0.8],
            tint,
            alpha: Alpha::Opaque,
            water: false,
            shade: true,
            emission: 0.0,
            cullface: None,
            ao: false,
        };
        orient_outward(&mut q, center);
        out.push(q);
    }
}

/// A display entity's orientation (`DisplayRenderer.calculateOrientation`):
/// its own yaw and pitch, or the camera's for the axes its `billboard` frees.
fn display_orientation(e: &Entity, view: Option<[f32; 2]>) -> Mat4 {
    let [cam_yaw, cam_pitch] = view.unwrap_or([e.yaw + 180.0, -e.pitch]);
    let (yaw, pitch) = match e.str_field("billboard").unwrap_or("fixed") {
        "vertical" => (cam_yaw - 180.0, e.pitch),
        "horizontal" => (e.yaw, -cam_pitch),
        "center" => (cam_yaw - 180.0, -cam_pitch),
        _ => (e.yaw, e.pitch),
    };
    // JOML rotationYXZ(-yaw, pitch, 0).
    Mat4::from_quat(
        Quat::from_rotation_y((-yaw).to_radians()) * Quat::from_rotation_x(pitch.to_radians()),
    )
}

/// A display entity's `transformation` (a matrix, or the decomposed form).
fn display_transformation(e: &Entity) -> Mat4 {
    match nbt::get(&e.nbt, "transformation") {
        Some(fastnbt::Value::List(l)) if l.len() == 16 => {
            let f: Vec<f32> = l.iter().filter_map(nbt::as_f64).map(|v| v as f32).collect();
            if let Ok(a) = <[f32; 16]>::try_from(f.as_slice()) {
                Mat4::from_cols_array(&a).transpose()
            } else {
                Mat4::IDENTITY
            }
        }
        Some(fastnbt::Value::Compound(c)) => {
            let v3 = |k: &str, d: f32| -> Vec3 {
                nbt::get(c, k)
                    .and_then(nbt::as_f64_list)
                    .filter(|v| v.len() == 3)
                    .map(|v| Vec3::new(v[0] as f32, v[1] as f32, v[2] as f32))
                    .unwrap_or(Vec3::splat(d))
            };
            let quat = |k: &str| -> Quat {
                match nbt::get(c, k) {
                    Some(fastnbt::Value::Compound(aa)) => {
                        // Axis-angle form {angle, axis: [x, y, z]}.
                        let angle = nbt::get_f64(aa, "angle").unwrap_or(0.0) as f32;
                        let axis = nbt::get(aa, "axis")
                            .and_then(nbt::as_f64_list)
                            .unwrap_or(vec![0.0, 1.0, 0.0]);
                        Quat::from_axis_angle(
                            Vec3::new(axis[0] as f32, axis[1] as f32, axis[2] as f32)
                                .normalize_or(Vec3::Y),
                            angle,
                        )
                    }
                    Some(v) => match nbt::as_f64_list(v) {
                        Some(q) if q.len() == 4 => {
                            Quat::from_xyzw(q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32)
                                .normalize()
                        }
                        _ => Quat::IDENTITY,
                    },
                    None => Quat::IDENTITY,
                }
            };
            Mat4::from_translation(v3("translation", 0.0))
                * Mat4::from_quat(quat("left_rotation"))
                * Mat4::from_scale(v3("scale", 1.0))
                * Mat4::from_quat(quat("right_rotation"))
        }
        _ => Mat4::IDENTITY,
    }
}

fn display_transform(e: &Entity, view: Option<[f32; 2]>) -> Mat4 {
    display_orientation(e, view) * display_transformation(e)
}

/// 3D direction from painting/item-frame NBT.
fn hanging_facing(e: &Entity) -> Vec3 {
    let three_d = e.num_field("facing").or_else(|| {
        // Item frames have always stored a 3D `Facing`; paintings used a 2D one before 1.21.
        if e.short_id().ends_with("item_frame") {
            e.num_field("Facing")
        } else {
            None
        }
    });
    if let Some(f) = three_d {
        // 1.21+: 3D data value (2 north, 3 south, 4 west, 5 east).
        return match f as i32 {
            2 => -Vec3::Z,
            3 => Vec3::Z,
            4 => -Vec3::X,
            5 => Vec3::X,
            0 => -Vec3::Y,
            _ => Vec3::Y,
        };
    }
    match e.num_field("Facing").unwrap_or(0.0) as i32 {
        0 => Vec3::Z,
        1 => -Vec3::X,
        2 => -Vec3::Z,
        _ => Vec3::X,
    }
}

fn flat_quad(p0: Vec3, eu: Vec3, ev: Vec3, sprite: &crate::assets::atlas::Sprite) -> Quad {
    Quad {
        p0,
        eu,
        ev,
        uv0: [sprite.x as f32, sprite.y as f32],
        uvs: [sprite.w as f32, 0.0],
        uvt: [0.0, sprite.h as f32],
        tint: [255; 4],
        alpha: Alpha::Cutout,
        water: false,
        shade: true,
        emission: 0.0,
        cullface: None,
        ao: false,
    }
}

pub fn build_entity_quads(
    scene: &mut Scene,
    models: &EntityModels,
    bakery: &mut ItemBakery,
    entities: &[Entity],
    store: &ResourceStore,
    opts: &super::BuildOptions,
) {
    let size = scene.volume.size_blocks();
    let mut quads: Vec<Quad> = Vec::new();
    let mut lights: Vec<EntityLight> = Vec::new();
    let mut stats = EntityStats::default();
    let mut scratch = Vec::new();
    for e in entities {
        let id = e.short_id().to_string();
        // Entity position relative to the scene origin (f64 first for precision).
        let rel = Vec3::new(
            (e.pos[0] - scene.origin[0] as f64) as f32,
            (e.pos[1] - scene.origin[1] as f64) as f32,
            (e.pos[2] - scene.origin[2] as f64) as f32,
        );
        if rel.x < 0.0 || rel.z < 0.0 || rel.x >= size[0] as f32 || rel.z >= size[2] as f32 {
            continue;
        }
        if is_invisible_kind(&id) {
            continue;
        }
        // An invisible body still shows what it holds and wears, as in vanilla.
        let invisible = e.flag("Invisible");

        if id == "item" {
            let drawn = match carried(e).into_iter().next() {
                Some(Carried::Item(stack, _)) => {
                    draw_dropped(bakery, scene, &mut stats, e, &stack, rel, &mut quads)
                }
                _ => false,
            };
            bump(
                if drawn {
                    &mut stats.rendered
                } else {
                    &mut stats.skipped
                },
                &id,
            );
            fill_lights(&quads, &mut lights, light_at(rel));
            continue;
        }
        if id == "item_display" {
            // A display with no item draws nothing, in the game too.
            let Some(Carried::Item(stack, ctx)) = carried(e).into_iter().next() else {
                continue;
            };
            // ItemDisplayRenderer: the display's pose, then a half turn about y.
            let m = Mat4::from_translation(rel)
                * display_transform(e, opts.view)
                * Mat4::from_rotation_y(std::f32::consts::PI);
            let drawn = draw_item(bakery, scene, &mut stats, &id, &stack, ctx, &m, &mut quads);
            bump(
                if drawn {
                    &mut stats.rendered
                } else {
                    &mut stats.skipped
                },
                &id,
            );
            fill_lights(
                &quads,
                &mut lights,
                light_at(m.transform_point3(Vec3::ZERO)),
            );
            continue;
        }

        if let Some(def) = models.models.get(&e.id) {
            let tex = entity_texture(def, e, store);
            let sprite = scene.atlas.sprite(&tex);
            let mut scale = def.scale;
            if e.num_field("Age").unwrap_or(0.0) < 0.0 || e.flag("IsBaby") {
                scale *= 0.5;
            }
            if id == "armor_stand" && e.flag("Small") {
                scale *= 0.5;
            }
            if id == "slime" || id == "magma_cube" {
                scale *= e.num_field("Size").unwrap_or(0.0) as f32 + 1.0;
            }
            // LivingEntityRenderer: translate(pos) rotY(180 - yaw) scale(-1,-1,1) scale(s) translate(0,-1.501,0)
            let root = Mat4::from_translation(rel)
                * Mat4::from_rotation_y((180.0 - e.yaw).to_radians())
                * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
                * Mat4::from_scale(Vec3::splat(scale))
                * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0));
            let stand = (id == "armor_stand").then(|| armor_stand_pose(e));
            let no_plate = e.flag("NoBasePlate");
            let show_arms = e.flag("ShowArms");
            let yaw = e.yaw.to_radians();
            // ArmorStandModel.setupAnim: the parts take the stand's `Pose`, the body
            // sticks follow the body, the arms show with `ShowArms`.
            let pose = |name: &str, p: &mut PartPose| {
                let Some(sp) = &stand else { return };
                let of = |n: &str| sp.iter().find(|(k, _)| *k == n).map(|(_, r)| *r);
                match name {
                    "base_plate" => {
                        p.rotation[1] -= yaw;
                        p.visible = !no_plate;
                    }
                    "left_body_stick" | "right_body_stick" | "shoulder_stick" => {
                        if let Some(r) = of("body") {
                            p.rotation = r;
                        }
                    }
                    _ => {
                        if let Some(r) = of(name) {
                            p.rotation = r;
                        }
                        if name.ends_with("_arm") {
                            p.visible = show_arms;
                        }
                    }
                }
            };
            let mut drew = false;
            if !invisible {
                for part in &def.parts {
                    part_quads(part, &root, &sprite, def.texture_size, &pose, &mut quads);
                }
                drew = true;
            }
            let light = light_at(rel + Vec3::new(0.0, def.hitbox[1] * scale * 0.5, 0.0));
            fill_lights(&quads, &mut lights, light);
            if let Some((family, shift)) = holder_kind(&id) {
                // A top-level part as posed for this frame.
                let posed = |name: &str| -> Option<(PartPose, [f32; 3])> {
                    let part = def.parts.iter().find(|p| p.name == name)?;
                    let mut pp = PartPose {
                        pivot: part.pivot,
                        rotation: part.rotation,
                        visible: true,
                    };
                    pose(name, &mut pp);
                    Some((pp, part.scale))
                };
                // The armour mesh copies the rotations of the body parts it covers.
                let armor_pose = |name: &str, p: &mut PartPose| {
                    if let Some((pp, _)) = posed(name) {
                        p.rotation = pp.rotation;
                    }
                };
                for c in carried(e) {
                    match c {
                        Carried::Armor(stack) => {
                            let Some((slot, layers)) = bakery.armor_layers(&stack) else {
                                continue;
                            };
                            if layers.is_empty() {
                                bump(&mut stats.items_skipped, short(&stack.id));
                                log::debug!("{id}: {} has no humanoid equipment layer", stack.id);
                                continue;
                            }
                            bakery.emit_armor(
                                scene,
                                family,
                                slot,
                                &layers,
                                &root,
                                &armor_pose,
                                &mut quads,
                            );
                            bump(&mut stats.items_rendered, short(&stack.id));
                            drew = true;
                        }
                        Carried::Item(stack, ctx) if ctx == DisplayContext::Head => {
                            // CustomHeadLayer.translateToHead, on the posed head.
                            let Some((head, sc)) = posed("head") else {
                                continue;
                            };
                            let m = root
                                * part_matrix(&head, sc)
                                * Mat4::from_translation(Vec3::new(0.0, -0.25, 0.0))
                                * Mat4::from_rotation_y(std::f32::consts::PI)
                                * Mat4::from_scale(Vec3::new(0.625, -0.625, -0.625));
                            drew |= draw_item(
                                bakery, scene, &mut stats, &id, &stack, ctx, &m, &mut quads,
                            );
                        }
                        Carried::Item(stack, ctx) => {
                            // ItemInHandLayer: translateToHand, then rotX(-90) rotY(180)
                            // translate(±1/16, 0.125, -0.625).
                            let right = ctx == DisplayContext::ThirdPersonRightHand;
                            let (arm, side) = if right {
                                ("right_arm", 1.0)
                            } else {
                                ("left_arm", -1.0)
                            };
                            let Some((mut pp, sc)) = posed(arm) else {
                                continue;
                            };
                            pp.pivot[0] += side * shift;
                            let m = root
                                * part_matrix(&pp, sc)
                                * Mat4::from_rotation_x((-90f32).to_radians())
                                * Mat4::from_rotation_y(std::f32::consts::PI)
                                * Mat4::from_translation(Vec3::new(side / 16.0, 0.125, -0.625));
                            drew |= draw_item(
                                bakery, scene, &mut stats, &id, &stack, ctx, &m, &mut quads,
                            );
                        }
                    }
                }
                fill_lights(&quads, &mut lights, light);
            }
            if drew {
                bump(&mut stats.rendered, &id);
            }
            continue;
        }

        if invisible && !matches!(id.as_str(), "item_frame" | "glow_item_frame") {
            continue;
        }
        let mut light_pos = rel + Vec3::new(0.0, 0.5, 0.0);
        if id == "block_display" {
            let Some(state) = nbt::get(&e.nbt, "block_state")
                .and_then(nbt::compound)
                .and_then(block_state_from_nbt)
            else {
                continue;
            };
            let m = Mat4::from_translation(rel) * display_transform(e, opts.view);
            push_block_model(scene, &state, &m, rel, &mut scratch, &mut quads);
            light_pos = m.transform_point3(Vec3::splat(0.5));
            bump(&mut stats.rendered, &id);
        } else if let Some(state) = item_frame_state(e) {
            // ItemFrameRenderer: from the entity, 0.46875 out along the facing to the
            // block centre, then turned so the frame's back (+z) is on the wall.
            let f = hanging_facing(e);
            let (xr, yr) = if f.y.abs() > 0.5 {
                (-90.0 * f.y.signum(), 180.0)
            } else {
                (0.0, 180.0 - (-f.x).atan2(f.z).to_degrees())
            };
            let frame = Mat4::from_translation(rel + f * 0.46875)
                * Mat4::from_rotation_x(f32::to_radians(xr))
                * Mat4::from_rotation_y(f32::to_radians(yr));
            light_pos = rel + f * 0.5;
            let mut drew = false;
            if !invisible {
                let m = frame * Mat4::from_translation(Vec3::splat(-0.5));
                push_block_model(scene, &state, &m, rel, &mut scratch, &mut quads);
                drew = true;
            }
            fill_lights(&quads, &mut lights, light_at(light_pos));
            if let Some(Carried::Item(stack, ctx)) = carried(e).into_iter().next() {
                // The item lies on the frame's face, turned by `ItemRotation` in
                // eighths, at half size, in the `fixed` context. A filled map's
                // picture needs the save's map data, which is not read: the map is
                // drawn as its item, as vanilla draws a map whose data is missing.
                let depth = if invisible { 0.5 } else { 0.4375 };
                let turns = e.num_field("ItemRotation").unwrap_or(0.0) as f32;
                let m = frame
                    * Mat4::from_translation(Vec3::new(0.0, 0.0, depth))
                    * Mat4::from_rotation_z((turns * 45.0).to_radians())
                    * Mat4::from_scale(Vec3::splat(0.5));
                drew |= draw_item(bakery, scene, &mut stats, &id, &stack, ctx, &m, &mut quads);
                // A glow frame's item is drawn at full brightness.
                let mut l = light_at(light_pos);
                l.full_bright = id == "glow_item_frame";
                fill_lights(&quads, &mut lights, l);
            }
            if drew {
                bump(&mut stats.rendered, &id);
            }
        } else if id == "painting" {
            let Some(var) = painting_variant(e) else {
                continue;
            };
            let sprite = scene.atlas.sprite(&var);
            let back = scene.atlas.sprite("minecraft:painting/back");
            let f = hanging_facing(e);
            let up = Vec3::Y;
            let right = (-f).cross(up).normalize();
            let (w, h) = (sprite.w as f32 / 16.0, sprite.h as f32 / 16.0);
            let c = rel;
            let front = flat_quad(
                c - right * (w / 2.0) + up * (h / 2.0) + f / 32.0,
                right * w,
                -up * h,
                &sprite,
            );
            let backq = flat_quad(
                c + right * (w / 2.0) + up * (h / 2.0) - f / 32.0,
                -right * w,
                -up * h,
                &back,
            );
            quads.push(front);
            quads.push(backq);
            light_pos = c + f * 0.5;
            bump(&mut stats.rendered, &id);
        } else if opts.unknown_entities_as_boxes {
            let [w, h] = fallback_hitbox(&id);
            let hsh = pos_hash(id.len() as i32, id.bytes().map(|b| b as i32).sum(), 7, 3);
            let tint = [
                96 + (hsh & 0x7F) as u8,
                96 + ((hsh >> 8) & 0x7F) as u8,
                96 + ((hsh >> 16) & 0x7F) as u8,
                255,
            ];
            let m = Mat4::from_translation(rel - Vec3::new(w / 2.0, 0.0, w / 2.0))
                * Mat4::from_scale(Vec3::new(w, h, w));
            box_quads(&m, &scene.atlas.sprite(WHITE_TEXTURE), tint, &mut quads);
            light_pos = rel + Vec3::new(0.0, h * 0.5, 0.0);
            bump(&mut stats.fallback_boxes, &id);
        } else {
            bump(&mut stats.skipped, &id);
        }
        fill_lights(&quads, &mut lights, light_at(light_pos));
    }
    if !stats.fallback_boxes.is_empty() {
        log::info!(
            "entities without a model (drawn as hitbox boxes): {:?}",
            stats.fallback_boxes
        );
    }
    if !stats.skipped.is_empty() {
        log::info!("entities skipped: {:?}", stats.skipped);
    }
    if !stats.items_skipped.is_empty() {
        log::info!("items not drawn: {:?}", stats.items_skipped);
    }
    log::info!(
        "entities rendered: {:?}; items rendered: {:?}",
        stats.rendered,
        stats.items_rendered
    );
    scene.entity_quads.extend(quads);
    scene.entity_quad_light.extend(lights);
    scene.entity_stats = stats;
}

/// An entity's equipment: `equipment` (1.21.5+), or the older `HandItems`
/// (main, off) and `ArmorItems` (feet, legs, chest, head).
#[derive(Default, Debug, Clone)]
pub struct Equipment {
    pub mainhand: Option<ItemStack>,
    pub offhand: Option<ItemStack>,
    pub head: Option<ItemStack>,
    pub chest: Option<ItemStack>,
    pub legs: Option<ItemStack>,
    pub feet: Option<ItemStack>,
}

pub fn equipment(e: &Entity) -> Equipment {
    let slot = |k: &str| nbt::get(&e.nbt, &format!("equipment.{k}")).and_then(ItemStack::from_nbt);
    let list = |k: &str, i: usize| match nbt::get(&e.nbt, k) {
        Some(fastnbt::Value::List(l)) => l.get(i).and_then(ItemStack::from_nbt),
        _ => None,
    };
    Equipment {
        mainhand: slot("mainhand").or_else(|| list("HandItems", 0)),
        offhand: slot("offhand").or_else(|| list("HandItems", 1)),
        feet: slot("feet").or_else(|| list("ArmorItems", 0)),
        legs: slot("legs").or_else(|| list("ArmorItems", 1)),
        chest: slot("chest").or_else(|| list("ArmorItems", 2)),
        head: slot("head").or_else(|| list("ArmorItems", 3)),
    }
}

/// Entities whose renderer has the armour, held-item and head-item layers: the
/// armour mesh family they wear, and the x shift (pixels, outward) their
/// `translateToHand` gives an arm (`SkeletonModel` moves it by one).
fn holder_kind(id: &str) -> Option<(&'static str, f32)> {
    Some(match id {
        "armor_stand" => ("armor_stand_armor", 0.0),
        "mannequin" | "player" | "zombie" | "husk" | "drowned" | "giant" | "zombie_villager"
        | "zombified_piglin" | "piglin" | "piglin_brute" => ("humanoid_armor", 0.0),
        "skeleton" | "stray" | "wither_skeleton" | "bogged" => ("humanoid_armor", 1.0),
        _ => return None,
    })
}

/// A stack an entity carries, with how it is drawn.
#[derive(Clone, Debug)]
pub enum Carried {
    /// Drawn through its item model in a display context.
    Item(ItemStack, DisplayContext),
    /// Worn: drawn through its equipment asset on the armour mesh.
    Armor(ItemStack),
}

/// Every stack an entity draws.
pub fn carried(e: &Entity) -> Vec<Carried> {
    let id = e.short_id();
    let mut out = Vec::new();
    match id {
        "item" => {
            if let Some(s) = nbt::get(&e.nbt, "Item").and_then(ItemStack::from_nbt) {
                out.push(Carried::Item(s, DisplayContext::Ground));
            }
        }
        "item_display" => {
            if let Some(s) = nbt::get(&e.nbt, "item").and_then(ItemStack::from_nbt) {
                out.push(Carried::Item(s, item_display_context(e)));
            }
        }
        "item_frame" | "glow_item_frame" => {
            if let Some(s) = nbt::get(&e.nbt, "Item").and_then(ItemStack::from_nbt) {
                out.push(Carried::Item(s, DisplayContext::Fixed));
            }
        }
        _ if holder_kind(id).is_some() => {
            let eq = equipment(e);
            let left_handed = e.flag("LeftHanded");
            let (right, left) = if left_handed {
                (eq.offhand, eq.mainhand)
            } else {
                (eq.mainhand, eq.offhand)
            };
            if let Some(s) = right {
                out.push(Carried::Item(s, DisplayContext::ThirdPersonRightHand));
            }
            if let Some(s) = left {
                out.push(Carried::Item(s, DisplayContext::ThirdPersonLeftHand));
            }
            if let Some(s) = eq.head {
                if super::items::armor_of(&s).is_some() {
                    out.push(Carried::Armor(s));
                } else {
                    out.push(Carried::Item(s, DisplayContext::Head));
                }
            }
            // Only armour is drawn from the body slots, as in vanilla.
            for s in [eq.chest, eq.legs, eq.feet].into_iter().flatten() {
                if super::items::armor_of(&s).is_some() {
                    out.push(Carried::Armor(s));
                }
            }
        }
        _ => {}
    }
    out
}

fn item_display_context(e: &Entity) -> DisplayContext {
    e.str_field("item_display")
        .and_then(DisplayContext::parse)
        .unwrap_or(DisplayContext::None)
}

/// Registers the textures of every stack the entities carry.
pub fn register_item_textures(bakery: &mut ItemBakery, entities: &[Entity], ab: &mut AtlasBuilder) {
    for e in entities {
        for c in carried(e) {
            match c {
                Carried::Item(stack, ctx) => {
                    let p = bakery.prepare(&stack, ctx);
                    bakery.register_textures(&p, ab);
                }
                Carried::Armor(stack) => {
                    if let Some((_, layers)) = bakery.armor_layers(&stack) {
                        bakery.register_armor(&layers, ab);
                    }
                }
            }
        }
    }
}

fn bump(m: &mut BTreeMap<String, usize>, k: &str) {
    *m.entry(k.to_string()).or_default() += 1;
}

fn short(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

/// Resolves and draws one stack posed by `m`; counts it either way.
#[allow(clippy::too_many_arguments)]
fn draw_item(
    bakery: &mut ItemBakery,
    scene: &Scene,
    stats: &mut EntityStats,
    holder: &str,
    stack: &ItemStack,
    ctx: DisplayContext,
    m: &Mat4,
    out: &mut Vec<Quad>,
) -> bool {
    let p = bakery.prepare(stack, ctx);
    count_item(stats, holder, stack, &p);
    if p.drawn() {
        bakery.emit(scene, &p, ctx, m, out);
    }
    p.drawn()
}

fn count_item(stats: &mut EntityStats, holder: &str, stack: &ItemStack, p: &Prepared) {
    if p.drawn() {
        bump(&mut stats.items_rendered, short(&stack.id));
        if !p.undrawn.is_empty() {
            log::debug!("{holder}: part of {} not drawn: {}", stack.id, p.reason());
        }
    } else {
        bump(&mut stats.items_skipped, short(&stack.id));
        log::debug!("{holder}: {} not drawn: {}", stack.id, p.reason());
    }
}

/// A float in [0, 1) from a hash of `seed` and `k`.
fn unit(seed: u32, k: u32) -> f32 {
    (pos_hash(seed as i32, k as i32, 0x17, 0x5EED) >> 8) as f32 / (1u32 << 24) as f32
}

fn str_hash(s: &str) -> u32 {
    s.bytes().fold(0x811C_9DC5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    })
}

/// A dropped item (`ItemEntityRenderer`): lifted so its model's lowest point
/// floats 1/16 above the entity, bobbing and spun by its phase, and drawn as a
/// cluster of up to five copies for a large stack. The phase vanilla draws at
/// random when the entity loads comes from its UUID here, and the clock is the
/// first frame after loading (age 0), so a frame is reproducible.
#[allow(clippy::too_many_arguments)]
fn draw_dropped(
    bakery: &mut ItemBakery,
    scene: &Scene,
    stats: &mut EntityStats,
    e: &Entity,
    stack: &ItemStack,
    rel: Vec3,
    out: &mut Vec<Quad>,
) -> bool {
    let p = bakery.prepare(stack, DisplayContext::Ground);
    count_item(stats, "item", stack, &p);
    if !p.drawn() {
        return false;
    }
    let mut probe = Vec::new();
    bakery.emit(
        scene,
        &p,
        DisplayContext::Ground,
        &Mat4::IDENTITY,
        &mut probe,
    );
    let Some((lo, hi)) = bounds(&probe) else {
        return false;
    };
    let uuid = match nbt::get(&e.nbt, "UUID") {
        Some(fastnbt::Value::IntArray(a)) => a.iter().fold(0u32, |h, &x| {
            pos_hash(h as i32, x, 0x55, 0x1D) ^ h.rotate_left(7)
        }),
        _ => str_hash(&format!("{:?}", e.pos)),
    };
    let phase = unit(uuid, 0) * std::f32::consts::TAU;
    let age = 0.0f32;
    let bob = (age / 10.0 + phase).sin() * 0.1 + 0.1;
    let lift = -lo.y + 0.0625;
    let spin = age / 20.0 + phase;
    let base =
        Mat4::from_translation(rel + Vec3::new(0.0, bob + lift, 0.0)) * Mat4::from_rotation_y(spin);
    let place = |m: &Mat4, out: &mut Vec<Quad>| {
        for q in &probe {
            let mut q = *q;
            transform_quad(&mut q, m);
            out.push(q);
        }
    };
    let n = rendered_amount(stack.count);
    let seed = str_hash(&stack.id);
    let jitter = |k: u32, axis: u32| (unit(seed, k * 3 + axis) * 2.0 - 1.0) * 0.15;
    let depth = hi.z - lo.z;
    if depth > 0.0625 {
        place(&base, out);
        for k in 1..n as u32 {
            let t = Vec3::new(jitter(k, 0), jitter(k, 1), jitter(k, 2));
            place(&(base * Mat4::from_translation(t)), out);
        }
    } else {
        let step = depth * 1.5;
        let mut m =
            base * Mat4::from_translation(Vec3::new(0.0, 0.0, -(step * (n - 1) as f32 / 2.0)));
        place(&m, out);
        m *= Mat4::from_translation(Vec3::new(0.0, 0.0, step));
        for k in 1..n as u32 {
            let t = Vec3::new(jitter(k, 0) * 0.5, jitter(k, 1) * 0.5, 0.0);
            place(&(m * Mat4::from_translation(t)), out);
            m *= Mat4::from_translation(Vec3::new(0.0, 0.0, step));
        }
    }
    true
}

/// An armour stand's part rotations in radians (`ArmorStand` `Pose`, with the
/// vanilla defaults for an absent part).
fn armor_stand_pose(e: &Entity) -> [(&'static str, [f32; 3]); 6] {
    let get = |key: &str, d: [f32; 3]| -> [f32; 3] {
        let deg = nbt::get(&e.nbt, &format!("Pose.{key}"))
            .and_then(nbt::as_f64_list)
            .filter(|v| v.len() == 3)
            .map(|v| [v[0] as f32, v[1] as f32, v[2] as f32])
            .unwrap_or(d);
        deg.map(f32::to_radians)
    };
    [
        ("head", get("Head", [0.0, 0.0, 0.0])),
        ("body", get("Body", [0.0, 0.0, 0.0])),
        ("left_arm", get("LeftArm", [-10.0, 0.0, -10.0])),
        ("right_arm", get("RightArm", [-15.0, 0.0, 10.0])),
        ("left_leg", get("LeftLeg", [-1.0, 0.0, -1.0])),
        ("right_leg", get("RightLeg", [1.0, 0.0, 1.0])),
    ]
}

fn fill_lights(quads: &[Quad], lights: &mut Vec<EntityLight>, light: EntityLight) {
    let n = quads.len() - lights.len();
    lights.extend(std::iter::repeat_n(light, n));
}

fn light_at(p: Vec3) -> EntityLight {
    let p = p.floor();
    EntityLight::shaded([p.x as i32, p.y as i32, p.z as i32])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_models_parse() {
        let m = EntityModels::load_builtin();
        assert_eq!(m.format, 1);
        for id in [
            "minecraft:pig",
            "minecraft:zombie",
            "minecraft:player",
            "minecraft:mannequin",
            "minecraft:armor_stand",
        ] {
            assert!(m.models.contains_key(id), "{id}");
        }
    }

    #[test]
    fn cube_faces_point_outward_and_cover_box() {
        let sprite = crate::assets::atlas::Sprite {
            x: 0,
            y: 0,
            w: 64,
            h: 64,
            transparent: false,
            translucent: false,
        };
        let c = CubeDef {
            uv: [0.0, 0.0],
            origin: [-4.0, -8.0, -4.0],
            size: [8.0, 8.0, 8.0],
            inflate: 0.0,
            mirror: false,
            faces: None,
        };
        for mirror in [false, true] {
            let mut out = Vec::new();
            cube_quads(
                &CubeDef {
                    mirror,
                    ..c.clone()
                },
                &Mat4::IDENTITY,
                &sprite,
                [64.0, 64.0],
                &mut out,
            );
            assert_eq!(out.len(), 6);
            let center = Vec3::new(0.0, -4.0, 0.0);
            for q in &out {
                let qc = q.p0 + (q.eu + q.ev) * 0.5;
                assert!(q.normal().dot(qc - center) > 0.0);
                assert!((q.eu.length() - 8.0).abs() < 1e-4 && (q.ev.length() - 8.0).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn pig_faces_south_at_yaw_zero() {
        // The pig's snout (most negative model z) must end up at +Z for yaw 0.
        let m = EntityModels::load_builtin();
        let pig = &m.models["minecraft:pig"];
        let sprite = crate::assets::atlas::Sprite {
            x: 0,
            y: 0,
            w: 64,
            h: 64,
            transparent: false,
            translucent: false,
        };
        let root = Mat4::from_rotation_y(180f32.to_radians())
            * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
            * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0));
        let mut out = Vec::new();
        let head = pig.parts.iter().find(|p| p.name == "head").unwrap();
        part_quads(head, &root, &sprite, pig.texture_size, &rest_pose, &mut out);
        let max_z = out
            .iter()
            .flat_map(|q| q.corners())
            .map(|c| c.z)
            .fold(f32::MIN, f32::max);
        assert!(max_z > 0.5, "snout at {max_z}");
        // Feet on the ground: every leg reaches y ~ 0.
        let mut all = Vec::new();
        for p in &pig.parts {
            part_quads(p, &root, &sprite, pig.texture_size, &rest_pose, &mut all);
        }
        let min_y = all
            .iter()
            .flat_map(|q| q.corners())
            .map(|c| c.y)
            .fold(f32::MAX, f32::min);
        assert!(min_y.abs() < 0.01, "min y {min_y}");
    }
}
