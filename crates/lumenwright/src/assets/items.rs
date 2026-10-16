//! Item model definitions (`assets/<ns>/items/<id>.json`, Minecraft 1.21.4+):
//! which model a stack draws in a display context, and with which tints.
//!
//! A definition is a tree of selectors (`condition`, `select`, `range_dispatch`)
//! over `model`, `composite`, `special` and `empty` leaves. Every selector is
//! evaluated from what the stack carries (its components) and the display
//! context; a property that depends on a live client (the player using the
//! item, the clock, the compass target, a cooldown) takes the value of an item
//! at rest: `false`, the fallback case, or 0.

use fastnbt::Value;
use serde_json::Value as Json;

use super::source::{ResourceStore, norm_loc};
use crate::world::nbt::{self, Compound};

/// Vanilla `ItemDisplayContext`: where an item is drawn, which picks the
/// model's `display` transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayContext {
    None,
    ThirdPersonLeftHand,
    ThirdPersonRightHand,
    FirstPersonLeftHand,
    FirstPersonRightHand,
    Head,
    Gui,
    Ground,
    Fixed,
    OnShelf,
}

impl DisplayContext {
    /// The context's name in model JSON and in `item_display` NBT.
    pub fn name(self) -> &'static str {
        match self {
            DisplayContext::None => "none",
            DisplayContext::ThirdPersonLeftHand => "thirdperson_lefthand",
            DisplayContext::ThirdPersonRightHand => "thirdperson_righthand",
            DisplayContext::FirstPersonLeftHand => "firstperson_lefthand",
            DisplayContext::FirstPersonRightHand => "firstperson_righthand",
            DisplayContext::Head => "head",
            DisplayContext::Gui => "gui",
            DisplayContext::Ground => "ground",
            DisplayContext::Fixed => "fixed",
            DisplayContext::OnShelf => "on_shelf",
        }
    }

    pub fn parse(s: &str) -> Option<DisplayContext> {
        const ALL: [DisplayContext; 10] = [
            DisplayContext::None,
            DisplayContext::ThirdPersonLeftHand,
            DisplayContext::ThirdPersonRightHand,
            DisplayContext::FirstPersonLeftHand,
            DisplayContext::FirstPersonRightHand,
            DisplayContext::Head,
            DisplayContext::Gui,
            DisplayContext::Ground,
            DisplayContext::Fixed,
            DisplayContext::OnShelf,
        ];
        ALL.into_iter().find(|c| c.name() == s)
    }

    /// Left-hand contexts mirror the transform (`ItemDisplayContext.leftHand`).
    pub fn left_hand(self) -> bool {
        matches!(
            self,
            DisplayContext::ThirdPersonLeftHand | DisplayContext::FirstPersonLeftHand
        )
    }
}

/// An item stack as a save stores it: `{id, count, components}` (1.20.5+), or
/// the older `{id, Count, tag}`.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemStack {
    /// Namespaced id, e.g. `minecraft:iron_sword`.
    pub id: String,
    pub count: i32,
    pub components: Compound,
}

impl ItemStack {
    /// Reads a stack; `None` for an empty slot (no id, air, or a count below 1).
    pub fn from_nbt(v: &Value) -> Option<ItemStack> {
        let c = nbt::compound(v)?;
        let id = norm_loc(nbt::get_str(c, "id")?);
        if id == "minecraft:air" {
            return None;
        }
        let count = nbt::get(c, "count")
            .or_else(|| nbt::get(c, "Count"))
            .and_then(nbt::as_i64)
            .unwrap_or(1) as i32;
        if count < 1 {
            return None;
        }
        let components = nbt::get(c, "components")
            .and_then(nbt::compound)
            .cloned()
            .unwrap_or_default();
        Some(ItemStack {
            id,
            count,
            components,
        })
    }

    /// A data component by name (`dyed_color` or `minecraft:dyed_color`).
    pub fn component(&self, name: &str) -> Option<&Value> {
        let full = norm_loc(name);
        self.components.get(&full).or_else(|| {
            let short = full.strip_prefix("minecraft:")?;
            self.components.get(short)
        })
    }

    fn component_i64(&self, name: &str) -> Option<i64> {
        self.component(name).and_then(nbt::as_i64)
    }
}

/// A tint a layer's `tintindex` takes (vanilla `ItemTintSource`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tint {
    /// 0xRRGGBB.
    Rgb(u32),
    /// `minecraft:grass`: the grass colormap at a climate.
    Grass { temperature: f32, downfall: f32 },
}

/// One drawable part of a resolved item.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemLayer {
    /// A JSON model (`models/<...>.json`) and the tints its tint indices take.
    Model { model: String, tints: Vec<Tint> },
    /// A model vanilla draws in Java code (`minecraft:special`): its `type`
    /// (`minecraft:shield`, `minecraft:chest`, ...) and the `base` model that
    /// carries the display transforms.
    Special { kind: String, base: String },
}

/// What a stack draws in one context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedItem {
    pub layers: Vec<ItemLayer>,
    /// Why part of the item cannot be resolved (no definition, an unknown type).
    pub unresolved: Vec<String>,
}

/// Resolves a stack's item definition for a display context. The definition is
/// `assets/<ns>/items/<path>.json` for the stack's id, or the one its
/// `item_model` component names.
pub fn resolve(store: &ResourceStore, stack: &ItemStack, ctx: DisplayContext) -> ResolvedItem {
    let mut out = ResolvedItem::default();
    let def_loc = stack
        .component("item_model")
        .and_then(nbt::as_str)
        .map(norm_loc)
        .unwrap_or_else(|| stack.id.clone());
    let Some(bytes) = store.resolve("items", &def_loc, "json") else {
        out.unresolved.push(format!("no item definition {def_loc}"));
        return out;
    };
    let json: Json = match serde_json::from_slice(bytes) {
        Ok(j) => j,
        Err(e) => {
            out.unresolved
                .push(format!("item definition {def_loc}: {e}"));
            return out;
        }
    };
    match json.get("model") {
        Some(m) => walk(m, stack, ctx, &mut out, 0),
        None => out
            .unresolved
            .push(format!("item definition {def_loc} has no model")),
    }
    out
}

fn walk(m: &Json, stack: &ItemStack, ctx: DisplayContext, out: &mut ResolvedItem, depth: u32) {
    if depth > 32 {
        out.unresolved.push("item model nested too deeply".into());
        return;
    }
    let ty = m
        .get("type")
        .and_then(Json::as_str)
        .map(norm_loc)
        .unwrap_or_default();
    let next = |key: &str, out: &mut ResolvedItem| {
        if let Some(n) = m.get(key) {
            walk(n, stack, ctx, out, depth + 1);
        }
    };
    match ty.as_str() {
        "minecraft:model" => {
            let Some(model) = m.get("model").and_then(Json::as_str) else {
                out.unresolved.push("model without a model".into());
                return;
            };
            let tints = m
                .get("tints")
                .and_then(Json::as_array)
                .map(|ts| ts.iter().map(|t| tint(t, stack)).collect())
                .unwrap_or_default();
            out.layers.push(ItemLayer::Model {
                model: norm_loc(model),
                tints,
            });
        }
        "minecraft:composite" => {
            for sub in m
                .get("models")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                walk(sub, stack, ctx, out, depth + 1);
            }
        }
        "minecraft:condition" => {
            let key = if condition(m, stack) {
                "on_true"
            } else {
                "on_false"
            };
            next(key, out);
        }
        "minecraft:select" => {
            let value = select_value(m, stack, ctx);
            let hit = value.as_deref().and_then(|v| {
                m.get("cases")?
                    .as_array()?
                    .iter()
                    .find(|c| match c.get("when") {
                        Some(Json::String(s)) => s == v || norm_loc(s) == v,
                        Some(Json::Array(a)) => a
                            .iter()
                            .filter_map(Json::as_str)
                            .any(|s| s == v || norm_loc(s) == v),
                        _ => false,
                    })
            });
            match hit.and_then(|c| c.get("model")) {
                Some(model) => walk(model, stack, ctx, out, depth + 1),
                None => next("fallback", out),
            }
        }
        "minecraft:range_dispatch" => {
            let scale = m.get("scale").and_then(Json::as_f64).unwrap_or(1.0) as f32;
            let value = range_value(m, stack) * scale;
            // The entry with the largest threshold not above the value.
            let mut best: Option<(f32, &Json)> = None;
            for e in m
                .get("entries")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
            {
                let t = e.get("threshold").and_then(Json::as_f64).unwrap_or(0.0) as f32;
                if t <= value
                    && best.is_none_or(|(bt, _)| t >= bt)
                    && let Some(model) = e.get("model")
                {
                    best = Some((t, model));
                }
            }
            match best {
                Some((_, model)) => walk(model, stack, ctx, out, depth + 1),
                None => next("fallback", out),
            }
        }
        "minecraft:special" => {
            let kind = m
                .get("model")
                .and_then(|s| s.get("type"))
                .and_then(Json::as_str)
                .map(norm_loc)
                .unwrap_or_default();
            let base = m
                .get("base")
                .and_then(Json::as_str)
                .map(norm_loc)
                .unwrap_or_default();
            out.layers.push(ItemLayer::Special { kind, base });
        }
        // An empty model, and the bundle's selected item (a live-inventory view):
        // nothing is drawn.
        "minecraft:empty" | "minecraft:bundle/selected_item" => {}
        other => out
            .unresolved
            .push(format!("unknown item model type {other:?}")),
    }
}

fn prop(m: &Json) -> String {
    m.get("property")
        .and_then(Json::as_str)
        .map(norm_loc)
        .unwrap_or_default()
}

/// `minecraft:custom_model_data` lists by kind (`flags`, `strings`, `floats`,
/// `colors`), at the selector's `index`.
fn custom_model_data<'a>(m: &Json, stack: &'a ItemStack, list: &str) -> Option<&'a Value> {
    let index = m.get("index").and_then(Json::as_u64).unwrap_or(0) as usize;
    match stack.component("custom_model_data").and_then(nbt::compound) {
        Some(c) => match c.get(list)? {
            Value::List(l) => l.get(index),
            _ => None,
        },
        None => None,
    }
}

fn condition(m: &Json, stack: &ItemStack) -> bool {
    match prop(m).as_str() {
        "minecraft:has_component" => m
            .get("component")
            .and_then(Json::as_str)
            .is_some_and(|c| stack.component(c).is_some()),
        "minecraft:damaged" => stack.component_i64("damage").unwrap_or(0) > 0,
        "minecraft:custom_model_data" => custom_model_data(m, stack, "flags")
            .and_then(nbt::as_i64)
            .is_some_and(|f| f != 0),
        // using_item, broken, selected, carried, extended_view, fishing_rod/cast,
        // bundle/has_selected_item, keybind_down, view_entity: an item at rest.
        _ => false,
    }
}

fn select_value(m: &Json, stack: &ItemStack, ctx: DisplayContext) -> Option<String> {
    match prop(m).as_str() {
        "minecraft:display_context" => Some(ctx.name().to_string()),
        "minecraft:trim_material" => stack
            .component("trim")
            .and_then(nbt::compound)
            .and_then(|t| nbt::get_str(t, "material"))
            .map(norm_loc),
        "minecraft:block_state" => {
            let key = m.get("block_state_property").and_then(Json::as_str)?;
            let state = stack.component("block_state").and_then(nbt::compound)?;
            nbt::get(state, key).and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                other => nbt::as_i64(other).map(|i| i.to_string()),
            })
        }
        "minecraft:charge_type" => {
            let loaded = match stack.component("charged_projectiles") {
                Some(Value::List(l)) => !l.is_empty(),
                _ => false,
            };
            Some(if loaded { "arrow" } else { "none" }.to_string())
        }
        "minecraft:main_hand" => Some("right".to_string()),
        "minecraft:custom_model_data" => {
            custom_model_data(m, stack, "strings").and_then(|v| nbt::as_str(v).map(str::to_string))
        }
        // context_dimension, context_entity_type, local_time, component: the
        // fallback (no world clock is read).
        _ => None,
    }
}

fn range_value(m: &Json, stack: &ItemStack) -> f32 {
    match prop(m).as_str() {
        "minecraft:count" => {
            let normalize = m.get("normalize").and_then(Json::as_bool).unwrap_or(true);
            let max = stack.component_i64("max_stack_size").unwrap_or(64).max(1) as f32;
            if normalize {
                (stack.count as f32 / max).clamp(0.0, 1.0)
            } else {
                stack.count as f32
            }
        }
        "minecraft:damage" => {
            let damage = stack.component_i64("damage").unwrap_or(0) as f32;
            let normalize = m.get("normalize").and_then(Json::as_bool).unwrap_or(true);
            match stack.component_i64("max_damage") {
                Some(max) if normalize && max > 0 => (damage / max as f32).clamp(0.0, 1.0),
                _ if normalize => 0.0,
                _ => damage,
            }
        }
        "minecraft:custom_model_data" => custom_model_data(m, stack, "floats")
            .and_then(nbt::as_f64)
            .unwrap_or(0.0) as f32,
        // compass, time, cooldown, crossbow/pull, use_duration, use_cycle,
        // bundle/fullness: an item at rest.
        _ => 0.0,
    }
}

fn json_rgb(v: Option<&Json>) -> Option<u32> {
    match v? {
        Json::Number(n) => n.as_i64().map(|i| i as u32 & 0xFF_FFFF),
        // An RGB float triple.
        Json::Array(a) if a.len() >= 3 => {
            let c = |i: usize| (a[i].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0) as u32;
            Some((c(0) << 16) | (c(1) << 8) | c(2))
        }
        _ => None,
    }
}

fn nbt_rgb(v: Option<&Value>) -> Option<u32> {
    match v? {
        // `dyed_color` before 1.21.5 was `{rgb: int}`.
        Value::Compound(c) => nbt::get(c, "rgb").and_then(nbt::as_i64),
        other => nbt::as_i64(other),
    }
    .map(|i| i as u32 & 0xFF_FFFF)
}

fn tint(t: &Json, stack: &ItemStack) -> Tint {
    let default = json_rgb(t.get("default")).unwrap_or(0xFF_FFFF);
    let ty = t
        .get("type")
        .and_then(Json::as_str)
        .map(norm_loc)
        .unwrap_or_default();
    Tint::Rgb(match ty.as_str() {
        "minecraft:constant" => json_rgb(t.get("value")).unwrap_or(0xFF_FFFF),
        "minecraft:dye" => nbt_rgb(stack.component("dyed_color")).unwrap_or(default),
        "minecraft:map_color" => nbt_rgb(stack.component("map_color")).unwrap_or(default),
        "minecraft:potion" => stack
            .component("potion_contents")
            .and_then(nbt::compound)
            .and_then(|c| nbt::get(c, "custom_color"))
            .and_then(nbt::as_i64)
            .map(|i| i as u32 & 0xFF_FFFF)
            .unwrap_or(default),
        "minecraft:custom_model_data" => custom_model_data(t, stack, "colors")
            .and_then(nbt::as_i64)
            .map(|i| i as u32 & 0xFF_FFFF)
            .unwrap_or(default),
        "minecraft:grass" => {
            return Tint::Grass {
                temperature: t.get("temperature").and_then(Json::as_f64).unwrap_or(0.5) as f32,
                downfall: t.get("downfall").and_then(Json::as_f64).unwrap_or(1.0) as f32,
            };
        }
        // firework, team: the default colour.
        _ => default,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(defs: &[(&str, &str)]) -> ResourceStore {
        let mut s = ResourceStore::new();
        for (id, json) in defs {
            s.insert(
                &format!("assets/minecraft/items/{id}.json"),
                json.as_bytes().to_vec(),
            );
        }
        s
    }

    fn stack(id: &str, components: Vec<(&str, Value)>) -> ItemStack {
        ItemStack {
            id: norm_loc(id),
            count: 1,
            components: components
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    fn model(m: &str) -> ItemLayer {
        ItemLayer::Model {
            model: m.into(),
            tints: vec![],
        }
    }

    #[test]
    fn stacks_read_both_formats_and_skip_empty_slots() {
        let mut c = Compound::new();
        c.insert("id".into(), Value::String("iron_sword".into()));
        c.insert("count".into(), Value::Int(3));
        let s = ItemStack::from_nbt(&Value::Compound(c.clone())).unwrap();
        assert_eq!((s.id.as_str(), s.count), ("minecraft:iron_sword", 3));
        c.remove("count");
        c.insert("Count".into(), Value::Byte(2));
        assert_eq!(ItemStack::from_nbt(&Value::Compound(c)).unwrap().count, 2);
        let mut air = Compound::new();
        air.insert("id".into(), Value::String("minecraft:air".into()));
        assert_eq!(ItemStack::from_nbt(&Value::Compound(air)), None);
        assert_eq!(ItemStack::from_nbt(&Value::Compound(Compound::new())), None);
    }

    #[test]
    fn select_takes_the_case_or_the_fallback() {
        let s = store(&[(
            "iron_helmet",
            r#"{"model":{"type":"minecraft:select","property":"minecraft:trim_material",
                "cases":[{"when":"minecraft:gold","model":{"type":"minecraft:model","model":"minecraft:item/gold_trim"}}],
                "fallback":{"type":"minecraft:model","model":"minecraft:item/iron_helmet"}}}"#,
        )]);
        let plain = resolve(&s, &stack("iron_helmet", vec![]), DisplayContext::Fixed);
        assert_eq!(plain.layers, vec![model("minecraft:item/iron_helmet")]);
        let mut trim = Compound::new();
        trim.insert("material".into(), Value::String("minecraft:gold".into()));
        let gold = resolve(
            &s,
            &stack(
                "iron_helmet",
                vec![("minecraft:trim", Value::Compound(trim))],
            ),
            DisplayContext::Fixed,
        );
        assert_eq!(gold.layers, vec![model("minecraft:item/gold_trim")]);
    }

    #[test]
    fn display_context_selects_and_conditions_rest() {
        let s = store(&[
            (
                "spyglass",
                r#"{"model":{"type":"minecraft:select","property":"minecraft:display_context",
                    "cases":[{"when":["gui","ground","fixed"],"model":{"type":"minecraft:model","model":"minecraft:item/spyglass"}}],
                    "fallback":{"type":"minecraft:model","model":"minecraft:item/spyglass_in_hand"}}}"#,
            ),
            (
                "shield",
                r#"{"model":{"type":"minecraft:condition","property":"minecraft:using_item",
                    "on_true":{"type":"minecraft:special","base":"minecraft:item/shield_blocking","model":{"type":"minecraft:shield"}},
                    "on_false":{"type":"minecraft:special","base":"minecraft:item/shield","model":{"type":"minecraft:shield"}}}}"#,
            ),
        ]);
        let g = resolve(&s, &stack("spyglass", vec![]), DisplayContext::Ground);
        assert_eq!(g.layers, vec![model("minecraft:item/spyglass")]);
        let h = resolve(
            &s,
            &stack("spyglass", vec![]),
            DisplayContext::ThirdPersonRightHand,
        );
        assert_eq!(h.layers, vec![model("minecraft:item/spyglass_in_hand")]);
        let sh = resolve(&s, &stack("shield", vec![]), DisplayContext::Fixed);
        assert_eq!(
            sh.layers,
            vec![ItemLayer::Special {
                kind: "minecraft:shield".into(),
                base: "minecraft:item/shield".into()
            }]
        );
    }

    #[test]
    fn range_dispatch_picks_the_highest_threshold_reached() {
        let s = store(&[(
            "clock",
            r#"{"model":{"type":"minecraft:range_dispatch","property":"minecraft:count","scale":64,
                "entries":[{"threshold":0,"model":{"type":"minecraft:model","model":"minecraft:item/a"}},
                           {"threshold":4,"model":{"type":"minecraft:model","model":"minecraft:item/b"}},
                           {"threshold":2,"model":{"type":"minecraft:model","model":"minecraft:item/c"}}]}}"#,
        )]);
        let mut st = stack("clock", vec![]);
        st.count = 3;
        assert_eq!(
            resolve(&s, &st, DisplayContext::Fixed).layers,
            vec![model("minecraft:item/c")]
        );
        st.count = 10;
        assert_eq!(
            resolve(&s, &st, DisplayContext::Fixed).layers,
            vec![model("minecraft:item/b")]
        );
    }

    #[test]
    fn tints_read_components_or_their_default() {
        let s = store(&[(
            "leather_helmet",
            r#"{"model":{"type":"minecraft:model","model":"minecraft:item/leather_helmet",
                "tints":[{"type":"minecraft:dye","default":-6265536},{"type":"minecraft:constant","value":-1},
                         {"type":"minecraft:grass","temperature":0.5,"downfall":1.0}]}}"#,
        )]);
        let undyed = resolve(&s, &stack("leather_helmet", vec![]), DisplayContext::Head);
        let tints = |r: &ResolvedItem| match &r.layers[0] {
            ItemLayer::Model { tints, .. } => tints.clone(),
            _ => panic!(),
        };
        assert_eq!(
            tints(&undyed),
            vec![
                Tint::Rgb(0xA06540),
                Tint::Rgb(0xFF_FFFF),
                Tint::Grass {
                    temperature: 0.5,
                    downfall: 1.0
                }
            ]
        );
        let dyed = resolve(
            &s,
            &stack(
                "leather_helmet",
                vec![("minecraft:dyed_color", Value::Int(0x3355AA))],
            ),
            DisplayContext::Head,
        );
        assert_eq!(tints(&dyed)[0], Tint::Rgb(0x3355AA));
    }

    #[test]
    fn a_missing_definition_is_named() {
        let s = store(&[]);
        let r = resolve(&s, &stack("nothing", vec![]), DisplayContext::Fixed);
        assert!(r.layers.is_empty());
        assert_eq!(r.unresolved, vec!["no item definition minecraft:nothing"]);
    }
}
