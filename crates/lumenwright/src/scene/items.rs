//! Item geometry. A stack resolves through its item definition
//! ([`crate::assets::items`]) to models; a model with elements bakes as a block
//! model does, and a flat item (`builtin/generated`) is extruded from its sprites
//! the way vanilla's `ItemModelGenerator` does. The model's display transform for
//! the context poses it. The shield, a model vanilla draws in Java code, is
//! data in `assets/item_models.json`, and so are the humanoid armour meshes that
//! equipment assets (`assets/<ns>/equipment/*.json`) are drawn on.

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat4, Vec3};

use super::entities::{EntityModels, ModelDef, PartPose, part_quads, rest_pose, transform_quad};
use super::{Quad, Scene};
use crate::assets::ResourceStore;
use crate::assets::atlas::{AtlasBuilder, load_texture};
use crate::assets::items::{self, DisplayContext, ItemLayer, ItemStack, Tint};
use crate::assets::model::{
    self, BakedQuad, ElementJson, FaceJson, ItemTransform, ResolvedModel, resolve_model,
};
use crate::assets::source::split_loc;

const BUILTIN_MODELS: &str = include_str!("../../assets/item_models.json");

/// The texture slots of a flat item, in vanilla's order (`ItemModelGenerator.LAYERS`).
const LAYERS: [&str; 5] = ["layer0", "layer1", "layer2", "layer3", "layer4"];

/// An item model ready to pose: quads in model space (0..1) and its display transforms.
#[derive(Debug)]
pub struct BakedItem {
    /// Quads in model space (0..1), as [`BakedQuad`]s with their `tintindex`.
    pub quads: Vec<BakedQuad>,
    /// Display transforms by context name (see [`DisplayContext::name`]); a context with no entry draws untransformed.
    pub display: HashMap<String, ItemTransform>,
}

impl BakedItem {
    /// The model's transform for a context, including the centring translate.
    pub fn transform(&self, ctx: DisplayContext) -> Mat4 {
        self.display
            .get(ctx.name())
            .copied()
            .unwrap_or(ItemTransform::IDENTITY)
            .matrix(ctx.left_hand())
    }
}

/// One drawable part of a prepared stack.
#[derive(Debug, Clone)]
pub enum Piece {
    /// A baked JSON model.
    Model {
        /// The baked model.
        item: Rc<BakedItem>,
        /// The tint each `tintindex` takes, by index; an index past the end is untinted.
        tints: Vec<Tint>,
    },
    /// `minecraft:shield`, posed by its base model's transforms.
    Shield {
        /// The base model whose `display` transforms pose the shield.
        base: Rc<BakedItem>,
    },
}

/// A stack resolved and baked for one display context.
#[derive(Debug, Clone, Default)]
pub struct Prepared {
    /// The parts to draw, in order; empty when nothing is drawn.
    pub pieces: Vec<Piece>,
    /// What could not be drawn, in words.
    pub undrawn: Vec<String>,
}

impl Prepared {
    /// Whether anything of the stack is drawn. A stack whose definition draws
    /// nothing (an `empty` model) is not.
    pub fn drawn(&self) -> bool {
        !self.pieces.is_empty()
    }

    /// Why nothing is drawn, for the log.
    pub fn reason(&self) -> String {
        if self.undrawn.is_empty() {
            "its item model draws nothing".to_string()
        } else {
            self.undrawn.join("; ")
        }
    }
}

/// Resolves, bakes and caches item models for one scene.
pub struct ItemBakery<'a> {
    store: &'a ResourceStore,
    baked: HashMap<String, Option<Rc<BakedItem>>>,
    specials: EntityModels,
}

/// Humanoid armour slots (vanilla `EquipmentSlot`'s armour half).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmorSlot {
    /// Helmet slot.
    Head,
    /// Chestplate slot.
    Chest,
    /// Leggings slot.
    Legs,
    /// Boots slot.
    Feet,
}

impl ArmorSlot {
    fn parse(s: &str) -> Option<ArmorSlot> {
        Some(match s {
            "head" => ArmorSlot::Head,
            "chest" => ArmorSlot::Chest,
            "legs" => ArmorSlot::Legs,
            "feet" => ArmorSlot::Feet,
            _ => return None,
        })
    }

    /// The parts of the armour mesh a slot keeps (`HumanoidModel.createArmorMeshSet`).
    pub fn parts(self) -> &'static [&'static str] {
        match self {
            ArmorSlot::Head => &["head"],
            ArmorSlot::Chest => &["body", "right_arm", "left_arm"],
            ArmorSlot::Legs => &["body", "right_leg", "left_leg"],
            ArmorSlot::Feet => &["right_leg", "left_leg"],
        }
    }

    /// The equipment layer type (`EquipmentClientInfo.LayerType`).
    fn layer_type(self) -> &'static str {
        match self {
            ArmorSlot::Legs => "humanoid_leggings",
            _ => "humanoid",
        }
    }

    /// Leggings use the inner mesh (deformation 0.5), the rest the outer (1.0).
    fn mesh(self) -> &'static str {
        match self {
            ArmorSlot::Legs => "inner",
            _ => "outer",
        }
    }
}

/// The equipment asset and slot of an armour stack: its `equippable` component,
/// or for a stack without one the vanilla armour items' defaults
/// (`Items` / `ArmorMaterials`, 1.21.11).
pub fn armor_of(stack: &ItemStack) -> Option<(String, ArmorSlot)> {
    if let Some(eq) = stack
        .component("equippable")
        .and_then(crate::world::nbt::compound)
    {
        let slot = crate::world::nbt::get_str(eq, "slot").and_then(ArmorSlot::parse)?;
        let asset = crate::world::nbt::get_str(eq, "asset_id")?;
        return Some((crate::assets::source::norm_loc(asset), slot));
    }
    let name = stack.id.strip_prefix("minecraft:")?;
    if name == "turtle_helmet" {
        return Some(("minecraft:turtle_scute".into(), ArmorSlot::Head));
    }
    let (material, piece) = name.rsplit_once('_')?;
    let slot = match piece {
        "helmet" => ArmorSlot::Head,
        "chestplate" => ArmorSlot::Chest,
        "leggings" => ArmorSlot::Legs,
        "boots" => ArmorSlot::Feet,
        _ => return None,
    };
    let asset = match material {
        "leather" | "chainmail" | "iron" | "diamond" | "netherite" | "copper" => material,
        "golden" => "gold",
        _ => return None,
    };
    Some((format!("minecraft:{asset}"), slot))
}

/// One texture layer of an equipment asset for a layer type, with the dye
/// colour it takes (`EquipmentClientInfo.Layer`).
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentLayer {
    /// `ns:entity/equipment/<type>/<path>`.
    pub texture: String,
    /// 0xRRGGBB, for a dyeable layer.
    pub dye: Option<u32>,
}

/// The layers `assets/<ns>/equipment/<asset>.json` gives a layer type.
pub fn equipment_layers(
    store: &ResourceStore,
    asset: &str,
    layer_type: &str,
    stack: &ItemStack,
) -> Vec<EquipmentLayer> {
    let Some(json) = store
        .resolve("equipment", asset, "json")
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
    else {
        return Vec::new();
    };
    let dyed = match stack.component("dyed_color") {
        Some(fastnbt::Value::Compound(c)) => {
            crate::world::nbt::get(c, "rgb").and_then(crate::world::nbt::as_i64)
        }
        Some(v) => crate::world::nbt::as_i64(v),
        None => None,
    };
    let list = json
        .get("layers")
        .and_then(|l| l.get(layer_type))
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter_map(|l| {
            let tex = l.get("texture")?.as_str()?;
            let (ns, path) = split_loc(tex);
            let dye = l.get("dyeable").map(|d| {
                dyed.or_else(|| d.get("color_when_undyed").and_then(|c| c.as_i64()))
                    .unwrap_or(0xFF_FFFF) as u32
                    & 0xFF_FFFF
            });
            Some(EquipmentLayer {
                texture: format!("{ns}:entity/equipment/{layer_type}/{path}"),
                dye,
            })
        })
        .collect()
}

/// Whether a texel of an RGBA8 image (red in the low byte) is fully transparent;
/// outside the image counts as transparent (`ItemModelGenerator.isTransparent`).
fn transparent(px: &[u32], w: u32, h: u32, x: i32, y: i32) -> bool {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        return true;
    }
    px[(y as u32 * w + x as u32) as usize] >> 24 == 0
}

/// The edge faces of a flat item layer: one quad for every edge of an opaque
/// texel whose neighbour is transparent (`ItemModelGenerator.createSideElements`).
/// Vanilla turns the left and right faces inward and lets the slab show them;
/// here every face points away from its texel, so the extrusion is a closed solid
/// that a one-sided rasteriser and a path tracer draw alike.
fn side_quads(texture: &str, tint: i32, w: u32, h: u32, px: &[u32]) -> Vec<BakedQuad> {
    let (sx, sy) = (16.0 / w as f32, 16.0 / h as f32);
    let (z0, dz) = (7.5 / 16.0, 1.0 / 16.0);
    let mut out = Vec::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if transparent(px, w, h, x, y) {
                continue;
            }
            // Model coordinates of the texel (texture v runs down, model y up).
            let x0 = x as f32 * sx / 16.0;
            let x1 = (x + 1) as f32 * sx / 16.0;
            let y_top = (16.0 - y as f32 * sy) / 16.0;
            let y_bot = (16.0 - (y + 1) as f32 * sy) / 16.0;
            // The texel's interior, inset by a tenth as vanilla does.
            let uv0 = [(x as f32 + 0.1) * sx, (y as f32 + 0.1) * sy];
            let (du, dv) = ([0.8 * sx, 0.0], [0.0, 0.8 * sy]);
            let mut face = |p0: Vec3, eu: Vec3, ev: Vec3| {
                out.push(BakedQuad {
                    p0,
                    eu,
                    ev,
                    texture: texture.to_string(),
                    uv0,
                    du,
                    dv,
                    cullface: None,
                    tintindex: tint,
                    shade: true,
                    light_emission: 0,
                });
            };
            let wx = Vec3::X * (sx / 16.0);
            let hy = Vec3::Y * (sy / 16.0);
            let tz = Vec3::Z * dz;
            if transparent(px, w, h, x, y - 1) {
                face(Vec3::new(x0, y_top, z0), wx, tz);
            }
            if transparent(px, w, h, x, y + 1) {
                face(Vec3::new(x0, y_bot, z0 + dz), wx, -tz);
            }
            if transparent(px, w, h, x - 1, y) {
                face(Vec3::new(x0, y_bot, z0), hy, tz);
            }
            if transparent(px, w, h, x + 1, y) {
                face(Vec3::new(x1, y_bot, z0), tz, hy);
            }
        }
    }
    out
}

/// A flat item's quads (`ItemModelGenerator`): per texture layer a front and a
/// back face across the whole sprite, 1/16 apart, and the edge faces.
fn generated_quads(store: &ResourceStore, model: &ResolvedModel) -> Vec<BakedQuad> {
    let mut out = Vec::new();
    for (i, layer) in LAYERS.iter().enumerate() {
        if !model.textures.contains_key(*layer) {
            break;
        }
        let texture = model.texture(&format!("#{layer}"));
        let face = |uv: [f32; 4]| FaceJson {
            uv: Some(uv),
            texture: format!("#{layer}"),
            cullface: None,
            rotation: 0,
            tintindex: i as i32,
        };
        let el = ElementJson {
            from: [0.0, 0.0, 7.5],
            to: [16.0, 16.0, 8.5],
            rotation: None,
            shade: true,
            light_emission: 0,
            faces: [
                ("south".to_string(), face([0.0, 0.0, 16.0, 16.0])),
                ("north".to_string(), face([16.0, 0.0, 0.0, 16.0])),
            ]
            .into_iter()
            .collect(),
        };
        let slab = ResolvedModel {
            elements: vec![el],
            textures: model.textures.clone(),
            ambient_occlusion: false,
            builtin: None,
            display: HashMap::new(),
        };
        out.extend(model::bake(&slab, 0, 0, false).quads);
        if let Some((w, h, mut px)) = load_texture(store, &texture) {
            // An animated sprite is a strip of square frames: frame 0.
            let h = if h > w && h % w == 0 {
                px.truncate((w * w) as usize);
                w
            } else {
                h
            };
            out.extend(side_quads(&texture, i as i32, w, h, &px));
        }
    }
    out
}

impl<'a> ItemBakery<'a> {
    /// A bakery that resolves and bakes models from `store`, with nothing cached yet.
    pub fn new(store: &'a ResourceStore) -> ItemBakery<'a> {
        ItemBakery {
            store,
            baked: HashMap::new(),
            specials: serde_json::from_str(BUILTIN_MODELS)
                .expect("bundled item_models.json is valid"),
        }
    }

    /// The resource store the bakery reads.
    pub fn store(&self) -> &'a ResourceStore {
        self.store
    }

    /// A bundled model (`assets/item_models.json`) by name.
    pub fn special_model(&self, name: &str) -> Option<&ModelDef> {
        self.specials.models.get(name)
    }

    fn model(&mut self, loc: &str) -> Option<Rc<BakedItem>> {
        let store = self.store;
        self.baked
            .entry(loc.to_string())
            .or_insert_with(|| {
                let m = resolve_model(store, loc)?;
                let quads = if m.builtin.as_deref() == Some("builtin/generated") {
                    generated_quads(store, &m)
                } else {
                    model::bake(&m, 0, 0, false).quads
                };
                Some(Rc::new(BakedItem {
                    quads,
                    display: m.display.clone(),
                }))
            })
            .clone()
    }

    /// Resolves and bakes a stack for a display context.
    pub fn prepare(&mut self, stack: &ItemStack, ctx: DisplayContext) -> Prepared {
        let resolved = items::resolve(self.store, stack, ctx);
        let mut out = Prepared {
            pieces: Vec::new(),
            undrawn: resolved.unresolved,
        };
        for layer in resolved.layers {
            match layer {
                ItemLayer::Model { model, tints } => match self.model(&model) {
                    Some(item) => out.pieces.push(Piece::Model { item, tints }),
                    None => out.undrawn.push(format!("no model {model}")),
                },
                ItemLayer::Special { kind, base } if kind == "minecraft:shield" => {
                    match self.model(&base) {
                        Some(base) => out.pieces.push(Piece::Shield { base }),
                        None => out.undrawn.push(format!("no model {base}")),
                    }
                }
                ItemLayer::Special { kind, .. } => {
                    out.undrawn.push(format!("special model {kind}"));
                }
            }
        }
        out
    }

    /// Registers the textures a prepared stack draws with.
    pub fn register_textures(&self, p: &Prepared, ab: &mut AtlasBuilder) {
        for piece in &p.pieces {
            match piece {
                Piece::Model { item, .. } => {
                    for q in &item.quads {
                        ab.add(&q.texture);
                    }
                }
                Piece::Shield { .. } => {
                    if let Some(def) = self.special_model("minecraft:shield") {
                        ab.add(&def.texture);
                    }
                }
            }
        }
    }

    /// Emits a prepared stack posed by `m` (the holder's pose, before the
    /// model's display transform).
    pub fn emit(
        &self,
        scene: &Scene,
        p: &Prepared,
        ctx: DisplayContext,
        m: &Mat4,
        out: &mut Vec<Quad>,
    ) {
        for piece in &p.pieces {
            match piece {
                Piece::Model { item, tints } => {
                    let t = *m * item.transform(ctx);
                    for bq in &item.quads {
                        let tint = usize::try_from(bq.tintindex)
                            .ok()
                            .and_then(|i| tints.get(i))
                            .map(|t| match *t {
                                Tint::Rgb(c) => c,
                                Tint::Grass {
                                    temperature,
                                    downfall,
                                } => scene.biomes.grass_climate(temperature, downfall),
                            })
                            .unwrap_or(0xFF_FFFF);
                        let mut q = scene.resolve_quad(bq, tint, false, false, 0);
                        transform_quad(&mut q, &t);
                        out.push(q);
                    }
                }
                Piece::Shield { base } => {
                    let Some(def) = self.special_model("minecraft:shield") else {
                        continue;
                    };
                    // ShieldSpecialRenderer: scale(1, -1, -1), then the model.
                    let t = *m * base.transform(ctx) * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0));
                    let sprite = scene.atlas.sprite(&def.texture);
                    for part in &def.parts {
                        part_quads(part, &t, &sprite, def.texture_size, &rest_pose, out);
                    }
                }
            }
        }
    }

    /// Emits one armour piece on a humanoid mesh. `mesh` names the bundled mesh
    /// family (`humanoid_armor` or `armor_stand_armor`); `pose` gives each part
    /// the rotation of the body part it covers.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_armor(
        &self,
        scene: &Scene,
        mesh: &str,
        slot: ArmorSlot,
        layers: &[EquipmentLayer],
        root: &Mat4,
        pose: &dyn Fn(&str, &mut PartPose),
        out: &mut Vec<Quad>,
    ) {
        let Some(def) = self.special_model(&format!("{mesh}_{}", slot.mesh())) else {
            return;
        };
        for (i, layer) in layers.iter().enumerate() {
            let sprite = scene.atlas.sprite(&layer.texture);
            let tint = layer.dye.unwrap_or(0xFF_FFFF);
            // Layers share one mesh; each later one sits a hair outside the last
            // so a rasteriser draws it over the one below, as vanilla's draw order does.
            let grow = i as f32 * 0.01;
            let before = out.len();
            for part in def
                .parts
                .iter()
                .filter(|p| slot.parts().contains(&p.name.as_str()))
            {
                let mut p = part.clone();
                if grow > 0.0 {
                    inflate(&mut p, grow);
                }
                if slot != ArmorSlot::Head {
                    p.children.clear();
                }
                part_quads(&p, root, &sprite, def.texture_size, pose, out);
            }
            for q in &mut out[before..] {
                q.tint = [(tint >> 16) as u8, (tint >> 8) as u8, tint as u8, 255];
            }
        }
    }

    /// Registers an armour piece's layer textures.
    pub fn register_armor(&self, layers: &[EquipmentLayer], ab: &mut AtlasBuilder) {
        for l in layers {
            ab.add(&l.texture);
        }
    }

    /// The layers an armour stack draws in its slot.
    pub fn armor_layers(&self, stack: &ItemStack) -> Option<(ArmorSlot, Vec<EquipmentLayer>)> {
        let (asset, slot) = armor_of(stack)?;
        Some((
            slot,
            equipment_layers(self.store, &asset, slot.layer_type(), stack),
        ))
    }
}

fn inflate(p: &mut super::entities::PartDef, by: f32) {
    for c in &mut p.cubes {
        c.inflate += by;
    }
    for ch in &mut p.children {
        inflate(ch, by);
    }
}

/// The model-space bounds of quads (`ItemStackRenderState.getModelBoundingBox`).
pub fn bounds(quads: &[Quad]) -> Option<(Vec3, Vec3)> {
    let mut it = quads.iter().flat_map(|q| q.corners());
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), c| (lo.min(c), hi.max(c))))
}

/// How many copies of a dropped stack vanilla draws (`ItemClusterRenderState.getRenderedAmount`).
pub fn rendered_amount(count: i32) -> usize {
    match count {
        i32::MIN..=1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, opaque: &[(u32, u32)]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for &(x, y) in opaque {
            img.put_pixel(x, y, image::Rgba([200, 100, 50, 255]));
        }
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    fn store_with_item(opaque: &[(u32, u32)]) -> ResourceStore {
        let mut s = ResourceStore::new();
        s.insert(
            "assets/minecraft/models/item/generated.json",
            br#"{"parent":"builtin/generated","display":{"ground":{"translation":[0,2,0],"scale":[0.5,0.5,0.5]},
               "thirdperson_righthand":{"rotation":[0,-90,55],"translation":[0,4,0.5]}}}"#
                .to_vec(),
        );
        s.insert(
            "assets/minecraft/models/item/thing.json",
            br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/thing"}}"#
                .to_vec(),
        );
        s.insert(
            "assets/minecraft/textures/item/thing.png",
            png(4, 4, opaque),
        );
        s
    }

    #[test]
    fn a_flat_item_is_a_slab_with_one_face_per_open_texel_edge() {
        // An L of three texels: (1,1), (1,2), (2,2).
        let s = store_with_item(&[(1, 1), (1, 2), (2, 2)]);
        let mut bakery = ItemBakery::new(&s);
        let item = bakery.model("minecraft:item/thing").unwrap();
        // Front and back, and the L's perimeter: 8 unit edges.
        assert_eq!(item.quads.len(), 2 + 8);
        for q in &item.quads[2..] {
            // Each edge face points away from the texel it belongs to: its centre
            // moved along the normal leaves the L's texel columns or rows.
            let n = q.normal();
            let c = q.p0 + (q.eu + q.ev) * 0.5;
            assert!(n.length() > 0.99 && n.z.abs() < 1e-6, "{n:?}");
            let probe = c + n * 0.01;
            let tx = (probe.x * 4.0).floor() as i32;
            let ty = ((1.0 - probe.y) * 4.0).floor() as i32;
            assert!(
                ![(1, 1), (1, 2), (2, 2)].contains(&(tx, ty)),
                "face at {c:?} normal {n:?} points into texel ({tx},{ty})"
            );
            // The face's texture is its own texel.
            let u = q.uv0[0] + 0.5 * (q.du[0] + q.dv[0]);
            let v = q.uv0[1] + 0.5 * (q.du[1] + q.dv[1]);
            let own = ((u / 4.0) as i32, (v / 4.0) as i32);
            assert!([(1, 1), (1, 2), (2, 2)].contains(&own), "{own:?}");
        }
        // The slab is 1/16 thick around z = 0.5.
        let zs: Vec<f32> = item.quads[..2].iter().map(|q| q.p0.z).collect();
        assert!(
            zs.contains(&(7.5 / 16.0)) && zs.contains(&(8.5 / 16.0)),
            "{zs:?}"
        );
    }

    #[test]
    fn display_transforms_resolve_through_parents_and_mirror_left() {
        let s = store_with_item(&[(0, 0)]);
        let mut bakery = ItemBakery::new(&s);
        let item = bakery.model("minecraft:item/thing").unwrap();
        // ground: translate (0, 2/16, 0), scale 0.5, after centring.
        let g = item.transform(DisplayContext::Ground);
        let p = g.transform_point3(Vec3::new(1.0, 1.0, 1.0));
        assert!(
            (p - Vec3::new(0.25, 0.125 + 0.25, 0.25)).length() < 1e-6,
            "{p:?}"
        );
        // The left hand takes the right hand's transform, mirrored: the left pose
        // of a point is the mirror image of the right pose of the point mirrored
        // across the model's centre plane x = 0.5.
        let r = item.transform(DisplayContext::ThirdPersonRightHand);
        let l = item.transform(DisplayContext::ThirdPersonLeftHand);
        let mirror = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let probe = Vec3::new(0.2, 0.7, 0.4);
        let a = l.transform_point3(probe);
        let expect = (mirror * r).transform_point3(Vec3::new(1.0 - probe.x, probe.y, probe.z));
        assert!((a - expect).length() < 1e-5, "{a:?} vs {expect:?}");
        // No transform for `none`: just the centring.
        let n = item.transform(DisplayContext::None);
        assert_eq!(n.transform_point3(Vec3::splat(0.5)), Vec3::ZERO);
    }

    #[test]
    fn armour_items_name_their_asset_and_slot() {
        let st = |id: &str| ItemStack {
            id: id.into(),
            count: 1,
            components: Default::default(),
        };
        assert_eq!(
            armor_of(&st("minecraft:iron_helmet")),
            Some(("minecraft:iron".into(), ArmorSlot::Head))
        );
        assert_eq!(
            armor_of(&st("minecraft:golden_leggings")),
            Some(("minecraft:gold".into(), ArmorSlot::Legs))
        );
        assert_eq!(
            armor_of(&st("minecraft:turtle_helmet")),
            Some(("minecraft:turtle_scute".into(), ArmorSlot::Head))
        );
        assert_eq!(armor_of(&st("minecraft:iron_sword")), None);
        assert_eq!(armor_of(&st("minecraft:carved_pumpkin")), None);
        assert_eq!(rendered_amount(1), 1);
        assert_eq!(rendered_amount(16), 2);
        assert_eq!(rendered_amount(64), 5);
    }
}
