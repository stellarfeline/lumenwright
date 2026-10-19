//! Entity geometry: data-driven cuboid mob models (`assets/entity_models.json`),
//! block displays, paintings and hitbox fallbacks.

use std::collections::{BTreeMap, HashMap};

use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;

use super::{Alpha, EntityLight, Quad, Scene};
use crate::assets::atlas::{AtlasBuilder, WHITE_TEXTURE};
use crate::assets::blockstate::pos_hash;
use crate::assets::source::{ResourceStore, norm_loc};
use crate::world::nbt::{self, Compound};
use crate::world::{BlockState, Entity};

const BUILTIN_MODELS: &str = include_str!("../../assets/entity_models.json");

#[derive(Deserialize, Clone, Debug)]
/// One cuboid of a model part, in vanilla's `CubeListBuilder` terms. Lengths are
/// model pixels (1/16 block).
pub struct CubeDef {
    /// Top-left corner of the cube's box-UV layout, in texture pixels.
    pub uv: [f32; 2],
    /// Minimum corner, relative to the part's pivot.
    pub origin: [f32; 3],
    /// Extent along x, y and z.
    pub size: [f32; 3],
    #[serde(default)]
    /// Grows the cube by this much on every side without changing its UVs.
    pub inflate: f32,
    #[serde(default)]
    /// Mirrors the box-UV layout left to right.
    pub mirror: bool,
    /// Faces to emit (vanilla `addBox(..., Set<Direction>)`), by direction name in
    /// model space; all six when absent.
    #[serde(default)]
    pub faces: Option<Vec<String>>,
}

#[derive(Deserialize, Clone, Debug)]
/// A model part in vanilla's `PartDefinition` terms: a pose, cubes and child parts.
pub struct PartDef {
    /// Part name, as vanilla names it; poses address parts by it.
    pub name: String,
    #[serde(default)]
    /// Pivot, in model pixels relative to the parent part.
    pub pivot: [f32; 3],
    #[serde(default)]
    /// Rotation about the pivot in radians, x, y, z, applied in Z-Y-X order.
    pub rotation: [f32; 3],
    /// Part scale (vanilla `PartPose.scaled`), applied after the rotation.
    #[serde(default = "unit_scale")]
    pub scale: [f32; 3],
    #[serde(default)]
    /// The part's cuboids.
    pub cubes: Vec<CubeDef>,
    #[serde(default)]
    /// Parts posed relative to this one.
    pub children: Vec<PartDef>,
}

#[derive(Deserialize, Clone, Debug)]
/// Picks an entity's texture from one of its NBT fields.
pub struct TextureVariants {
    /// NBT field whose string value names the variant.
    pub field: String,
    /// Texture location with `{name}` standing for the value's path (its namespace dropped).
    pub pattern: String,
}

#[derive(Deserialize, Clone, Debug)]
/// One entity model: its texture, its scale and hitbox, and its parts.
pub struct ModelDef {
    /// Vanilla class and method the geometry is transcribed from.
    #[serde(default)]
    pub source: Option<String>,
    /// Default texture; block-entity models pick theirs in code and leave it empty.
    #[serde(default)]
    pub texture: String,
    /// Texture picked from the entity's NBT; the default texture when absent or unset.
    pub texture_variants: Option<TextureVariants>,
    /// Texture width and height in pixels, the space the cube UVs are in.
    pub texture_size: [f32; 2],
    #[serde(default = "one")]
    /// Uniform scale of the whole model (halved again for babies and small armour stands).
    pub scale: f32,
    #[serde(default = "default_hitbox")]
    /// Hitbox width and height in blocks; the quick-mode light sample sits at half
    /// the height.
    pub hitbox: [f32; 2],
    /// The root parts.
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
/// The entity models bundled with the crate (`assets/entity_models.json`), by entity id.
pub struct EntityModels {
    /// Version of the file format.
    pub format: u32,
    /// Model definitions by namespaced entity id.
    pub models: HashMap<String, ModelDef>,
}

impl EntityModels {
    /// Parses the bundled model file.
    pub fn load_builtin() -> EntityModels {
        serde_json::from_str(BUILTIN_MODELS).expect("bundled entity_models.json is valid")
    }
}

#[derive(Default, Debug, Clone)]
/// Per-kind counts of the entities a scene drew.
pub struct EntityStats {
    /// Entities drawn from a model, as block displays, item frames or paintings.
    pub rendered: BTreeMap<String, usize>,
    /// Entities with no model drawn as a box the size of their hitbox.
    pub fallback_boxes: BTreeMap<String, usize>,
    /// Entities with no model left out, because fallback boxes are off.
    pub skipped: BTreeMap<String, usize>,
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
    // Mirroring transforms flip the winding; undo that so normals stay outward.
    let mirrored = m.determinant() < 0.0;
    for &mid in scratch.iter() {
        let model = &scene.baked[mid as usize];
        for bq in &model.quads {
            let mut q = scene.resolve_quad(bq, 0xFFFFFF, false, info.translucent, info.emission);
            let p0 = m.transform_point3(q.p0);
            q.eu = m.transform_vector3(q.eu);
            q.ev = m.transform_vector3(q.ev);
            q.p0 = p0;
            q.cullface = None;
            if mirrored {
                q.p0 += q.eu;
                q.eu = -q.eu;
                q.uv0 = [q.uv0[0] + q.uvs[0], q.uv0[1] + q.uvs[1]];
                q.uvs = [-q.uvs[0], -q.uvs[1]];
            }
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

/// Adds every texture the listed entities may sample to the atlas: model
/// textures, NBT-picked variants, profile skins and painting art. Block-display
/// textures come with the baked block models.
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
    /// Pivot, in model pixels relative to the parent part.
    pub pivot: [f32; 3],
    /// Rotation about the pivot in radians, x, y, z.
    pub rotation: [f32; 3],
    /// Whether the part and its children are drawn.
    pub visible: bool,
}

/// Sets a part's pose by name; the identity leaves the pose of the model file.
pub type PoseFn<'a> = &'a dyn Fn(&str, &mut PartPose);

/// The pose that leaves every part as the model file has it.
pub fn rest_pose(_: &str, _: &mut PartPose) {}

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
    let rot = p.rotation;
    // PoseStack: translate(pivot / 16), rotate ZYX, scale; cube vertices in pixels / 16.
    let local = Mat4::from_translation(Vec3::from(p.pivot) / 16.0)
        * Mat4::from_quat(Quat::from_euler(
            glam::EulerRot::ZYX,
            rot[2],
            rot[1],
            rot[0],
        ))
        * Mat4::from_scale(Vec3::from(part.scale));
    let m = *parent * local;
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

fn display_transform(e: &Entity) -> Mat4 {
    let rot = Mat4::from_quat(
        Quat::from_rotation_y((-e.yaw).to_radians()) * Quat::from_rotation_x(e.pitch.to_radians()),
    );
    let t = match nbt::get(&e.nbt, "transformation") {
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
    };
    rot * t
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

/// Appends the quads of every visible entity inside the volume to
/// `scene.entity_quads` (scene coordinates), with their quick-mode lighting in
/// `scene.entity_quad_light`, and records the counts in `scene.entity_stats`.
pub fn build_entity_quads(
    scene: &mut Scene,
    models: &EntityModels,
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
        if is_invisible_kind(&id) || e.flag("Invisible") {
            continue;
        }
        let before = quads.len();
        let mut light_pos = rel + Vec3::new(0.0, 0.5, 0.0);

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
            let armor_stand = id == "armor_stand";
            let no_plate = e.flag("NoBasePlate");
            let yaw = e.yaw.to_radians();
            let pose = |name: &str, p: &mut PartPose| {
                if armor_stand && name == "base_plate" {
                    p.rotation[1] -= yaw;
                    p.visible = !no_plate;
                }
            };
            for part in &def.parts {
                part_quads(part, &root, &sprite, def.texture_size, &pose, &mut quads);
            }
            light_pos = rel + Vec3::new(0.0, def.hitbox[1] * scale * 0.5, 0.0);
            *stats.rendered.entry(id.clone()).or_default() += 1;
        } else if id == "block_display" {
            let Some(state) = nbt::get(&e.nbt, "block_state")
                .and_then(nbt::compound)
                .and_then(block_state_from_nbt)
            else {
                continue;
            };
            let m = Mat4::from_translation(rel) * display_transform(e);
            push_block_model(scene, &state, &m, rel, &mut scratch, &mut quads);
            light_pos = m.transform_point3(Vec3::splat(0.5));
            *stats.rendered.entry(id.clone()).or_default() += 1;
        } else if let Some(state) = item_frame_state(e) {
            // The vanilla frame model sits against the block's south face, facing
            // north; rotate it so it faces out of the wall it hangs on.
            let f = hanging_facing(e);
            let rot = if f.y > 0.5 {
                Mat4::from_rotation_x(90f32.to_radians())
            } else if f.y < -0.5 {
                Mat4::from_rotation_x(-90f32.to_radians())
            } else {
                Mat4::from_rotation_y((-f.x).atan2(-f.z))
            };
            let m = Mat4::from_translation(rel)
                * rot
                * Mat4::from_translation(-Vec3::new(0.5, 0.5, 15.75 / 16.0));
            push_block_model(scene, &state, &m, rel, &mut scratch, &mut quads);
            light_pos = rel + f * 0.5;
            *stats.rendered.entry(id.clone()).or_default() += 1;
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
            *stats.rendered.entry(id.clone()).or_default() += 1;
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
            *stats.fallback_boxes.entry(id.clone()).or_default() += 1;
        } else {
            *stats.skipped.entry(id.clone()).or_default() += 1;
        }
        let lp = light_pos.floor();
        lights.extend(std::iter::repeat_n(
            EntityLight::shaded([lp.x as i32, lp.y as i32, lp.z as i32]),
            quads.len() - before,
        ));
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
    log::info!("entities rendered: {:?}", stats.rendered);
    scene.entity_quads.extend(quads);
    scene.entity_quad_light.extend(lights);
    scene.entity_stats = stats;
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
