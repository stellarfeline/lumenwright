//! Entity records from `entities/*.mca` (1.17+) and legacy in-chunk lists.

use fastnbt::Value;

use super::nbt::{self, Compound};

#[derive(Clone, Debug)]
/// One entity read from the save.
pub struct Entity {
    /// Namespaced id, e.g. `minecraft:pig`.
    pub id: String,
    /// Position in world coordinates (blocks).
    pub pos: [f64; 3],
    /// Yaw in degrees, Minecraft convention (0 = south, 90 = west).
    pub yaw: f32,
    /// Pitch in degrees; positive looks down.
    pub pitch: f32,
    /// The full entity compound, for model-specific fields (variants, facing...).
    pub nbt: Compound,
}

impl Entity {
    /// The id without the `minecraft:` namespace.
    pub fn short_id(&self) -> &str {
        self.id.strip_prefix("minecraft:").unwrap_or(&self.id)
    }

    /// A string field of the entity compound; `key` may be a dotted path.
    pub fn str_field(&self, key: &str) -> Option<&str> {
        nbt::get_str(&self.nbt, key)
    }

    /// A numeric field of the entity compound as `f64`; `key` may be a dotted path.
    pub fn num_field(&self, key: &str) -> Option<f64> {
        nbt::get_f64(&self.nbt, key)
    }

    /// Whether a numeric field is present and non-zero; `key` may be a dotted path.
    pub fn flag(&self, key: &str) -> bool {
        nbt::get_bool(&self.nbt, key)
    }
}

/// Decodes one entity compound, flattening passengers into `out`.
pub fn collect_entity(v: &Value, out: &mut Vec<Entity>) {
    let Some(c) = nbt::compound(v) else { return };
    let id = nbt::get_str(c, "id").unwrap_or("").to_string();
    if let Some(pos) = nbt::get(c, "Pos").and_then(nbt::as_f64_list)
        && pos.len() == 3
        && !id.is_empty()
    {
        let rot = nbt::get(c, "Rotation")
            .and_then(nbt::as_f64_list)
            .unwrap_or_default();
        out.push(Entity {
            id,
            pos: [pos[0], pos[1], pos[2]],
            yaw: rot.first().copied().unwrap_or(0.0) as f32,
            pitch: rot.get(1).copied().unwrap_or(0.0) as f32,
            nbt: c.clone(),
        });
    }
    if let Some(Value::List(passengers)) = c.get("Passengers") {
        for p in passengers {
            collect_entity(p, out);
        }
    }
}

/// Decodes an entity-region chunk (`{DataVersion, Position, Entities}`).
pub fn parse_entity_chunk(nbt_bytes: &[u8]) -> anyhow::Result<Vec<Entity>> {
    let root: Value = fastnbt::from_bytes(nbt_bytes)?;
    let mut out = Vec::new();
    if let Some(Value::List(list)) = nbt::compound(&root).and_then(|c| c.get("Entities")) {
        for e in list {
            collect_entity(e, &mut out);
        }
    }
    Ok(out)
}
