//! Blockstate JSON (`variants` / `multipart`) and per-state model selection.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use super::model::{self, BakedModel};
use super::source::{ResourceStore, norm_loc};
use crate::world::BlockState;

#[derive(Deserialize, Clone, Debug)]
pub struct VariantJson {
    pub model: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub uvlock: bool,
    #[serde(default = "one")]
    pub weight: u32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T: Clone> OneOrMany<T> {
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            OneOrMany::One(t) => vec![t.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct MultipartCase {
    pub when: Option<Value>,
    pub apply: OneOrMany<VariantJson>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BlockStateJson {
    pub variants: Option<serde_json::Map<String, Value>>,
    pub multipart: Option<Vec<MultipartCase>>,
}

fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Evaluates a multipart `when` clause.
pub fn when_matches(when: &Value, state: &BlockState) -> bool {
    let Value::Object(map) = when else {
        return true;
    };
    if let Some(Value::Array(or)) = map.get("OR") {
        return or.iter().any(|c| when_matches(c, state));
    }
    if let Some(Value::Array(and)) = map.get("AND") {
        return and.iter().all(|c| when_matches(c, state));
    }
    map.iter().all(|(k, v)| {
        let want = value_str(v);
        let (negate, want) = match want.strip_prefix('!') {
            Some(rest) => (true, rest.to_string()),
            None => (false, want),
        };
        let have = state.prop(k).unwrap_or("");
        let hit = want.split('|').any(|w| w == have);
        hit != negate
    })
}

/// Tests a `variants` key like `facing=east,half=bottom` against a state.
pub fn variant_key_matches(key: &str, state: &BlockState) -> bool {
    if key.is_empty() || key == "normal" {
        return true;
    }
    key.split(',').all(|kv| match kv.split_once('=') {
        Some((k, v)) => state.prop(k) == Some(v),
        None => true,
    })
}

/// Models making up one block state: each part is a weighted list of baked models.
#[derive(Clone, Debug, Default)]
pub struct BlockModelSet {
    pub parts: Vec<Vec<(u32, u32)>>,
    pub missing: bool,
}

#[inline]
pub fn pos_hash(x: i32, y: i32, z: i32, salt: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ (z as u32).wrapping_mul(0xCB1A_B31F);
    h ^= salt.wrapping_mul(0x9E37_79B9);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

impl BlockModelSet {
    /// Picks one model per part, deterministically by position.
    pub fn choose(&self, x: i32, y: i32, z: i32, out: &mut Vec<u32>) {
        out.clear();
        for (i, part) in self.parts.iter().enumerate() {
            if part.len() == 1 {
                out.push(part[0].0);
                continue;
            }
            let total: u32 = part.iter().map(|p| p.1.max(1)).sum();
            let mut r = pos_hash(x, y, z, i as u32) % total.max(1);
            for &(id, w) in part {
                let w = w.max(1);
                if r < w {
                    out.push(id);
                    break;
                }
                r -= w;
            }
        }
    }

    pub fn is_deterministic(&self) -> bool {
        self.parts.iter().all(|p| p.len() == 1)
    }
}

/// Caches resolved and baked models.
pub struct ModelBakery<'a> {
    pub store: &'a ResourceStore,
    resolved: HashMap<String, Option<model::ResolvedModel>>,
    baked_ids: HashMap<(String, i32, i32, bool), u32>,
    pub baked: Vec<BakedModel>,
    blockstates: HashMap<String, Option<BlockStateJson>>,
}

impl<'a> ModelBakery<'a> {
    pub fn new(store: &'a ResourceStore) -> ModelBakery<'a> {
        let mut b = ModelBakery {
            store,
            resolved: HashMap::new(),
            baked_ids: HashMap::new(),
            baked: Vec::new(),
            blockstates: HashMap::new(),
        };
        // Baked id 0: the vanilla "missing" cube.
        b.baked.push(model::cube_all(model::MISSING_TEXTURE));
        b
    }

    pub const MISSING: u32 = 0;

    pub fn bake(&mut self, loc: &str, x: i32, y: i32, uvlock: bool) -> u32 {
        let loc = norm_loc(loc);
        let key = (loc.clone(), x, y, uvlock);
        if let Some(&id) = self.baked_ids.get(&key) {
            return id;
        }
        let resolved = self
            .resolved
            .entry(loc.clone())
            .or_insert_with(|| model::resolve_model(self.store, &loc))
            .clone();
        let id = match resolved {
            Some(m) => {
                self.baked.push(model::bake(&m, x, y, uvlock));
                (self.baked.len() - 1) as u32
            }
            None => {
                log::debug!("missing model {loc}");
                Self::MISSING
            }
        };
        self.baked_ids.insert(key, id);
        id
    }

    fn blockstate(&mut self, name: &str) -> Option<BlockStateJson> {
        let store = self.store;
        self.blockstates
            .entry(name.to_string())
            .or_insert_with(|| {
                let bytes = store.resolve("blockstates", name, "json")?;
                match serde_json::from_slice(bytes) {
                    Ok(b) => Some(b),
                    Err(e) => {
                        log::warn!("blockstate {name}: {e}");
                        None
                    }
                }
            })
            .clone()
    }

    /// Resolves the model set for a block state.
    pub fn models_for_state(&mut self, state: &BlockState) -> BlockModelSet {
        let Some(bs) = self.blockstate(&state.name) else {
            return BlockModelSet {
                parts: vec![vec![(Self::MISSING, 1)]],
                missing: true,
            };
        };
        let mut parts = Vec::new();
        if let Some(variants) = &bs.variants {
            let chosen = variants
                .iter()
                .find(|(k, _)| variant_key_matches(k, state))
                .map(|(_, v)| v.clone());
            if let Some(v) = chosen
                && let Ok(list) = serde_json::from_value::<OneOrMany<VariantJson>>(v)
            {
                let part: Vec<(u32, u32)> = list
                    .to_vec()
                    .iter()
                    .map(|v| (self.bake(&v.model, v.x, v.y, v.uvlock), v.weight))
                    .collect();
                parts.push(part);
            }
        }
        if let Some(cases) = &bs.multipart {
            for c in cases {
                if c.when.as_ref().is_none_or(|w| when_matches(w, state)) {
                    let part: Vec<(u32, u32)> = c
                        .apply
                        .to_vec()
                        .iter()
                        .map(|v| (self.bake(&v.model, v.x, v.y, v.uvlock), v.weight))
                        .collect();
                    parts.push(part);
                }
            }
        }
        let missing = parts.is_empty();
        if missing {
            parts.push(vec![(Self::MISSING, 1)]);
        }
        BlockModelSet { parts, missing }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_keys() {
        let s = BlockState::new("minecraft:oak_stairs")
            .with("facing", "east")
            .with("half", "bottom");
        assert!(variant_key_matches("facing=east,half=bottom", &s));
        assert!(!variant_key_matches("facing=west,half=bottom", &s));
        assert!(variant_key_matches("", &s));
    }

    #[test]
    fn multipart_when() {
        let s = BlockState::new("minecraft:oak_fence")
            .with("north", "true")
            .with("east", "false");
        assert!(when_matches(&serde_json::json!({"north": "true"}), &s));
        assert!(when_matches(&serde_json::json!({"north": true}), &s));
        assert!(!when_matches(&serde_json::json!({"east": "true"}), &s));
        assert!(when_matches(
            &serde_json::json!({"OR": [{"east": "true"}, {"north": "true"}]}),
            &s
        ));
        assert!(!when_matches(
            &serde_json::json!({"AND": [{"east": "true"}, {"north": "true"}]}),
            &s
        ));
        assert!(when_matches(&serde_json::json!({"east": "none|false"}), &s));
        assert!(when_matches(&serde_json::json!({"east": "!true"}), &s));
    }

    #[test]
    fn weighted_choice_is_deterministic_and_covers_all() {
        let set = BlockModelSet {
            parts: vec![vec![(1, 1), (2, 1), (3, 2)]],
            missing: false,
        };
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for x in 0..200 {
            set.choose(x, 64, -x, &mut out);
            seen.insert(out[0]);
            let first = out[0];
            set.choose(x, 64, -x, &mut out);
            assert_eq!(out[0], first);
        }
        assert_eq!(seen.len(), 3);
    }
}
