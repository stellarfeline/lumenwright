//! Per-block properties that vanilla hard-codes in Java (not in assets):
//! light emission, tint sources, fluids and same-block face culling.

use crate::world::BlockState;

/// Where a face with `tintindex >= 0` takes its colour from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintKind {
    /// Untinted.
    None,
    /// The biome's grass colour.
    Grass,
    /// The biome's foliage colour.
    Foliage,
    /// The biome's dry-foliage colour.
    DryFoliage,
    /// The biome's water colour.
    Water,
    /// A fixed colour, `0xRRGGBB`, whatever the biome.
    Fixed(u32),
}

fn name(s: &BlockState) -> &str {
    s.short_name()
}

/// Where the tinted faces of a block state take their colour from.
pub fn tint_kind(s: &BlockState) -> TintKind {
    let n = name(s);
    match n {
        "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "potted_fern"
        | "sugar_cane" | "bush" | "pink_petals" | "wildflowers" => TintKind::Grass,
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" | "vine" => TintKind::Foliage,
        "leaf_litter" => TintKind::DryFoliage,
        "birch_leaves" => TintKind::Fixed(0x80A755),
        "spruce_leaves" => TintKind::Fixed(0x619961),
        "lily_pad" => TintKind::Fixed(0x208030),
        "attached_melon_stem" | "attached_pumpkin_stem" => TintKind::Fixed(0xE0C71C),
        "melon_stem" | "pumpkin_stem" => {
            let age: u32 = s.prop("age").and_then(|a| a.parse().ok()).unwrap_or(0);
            TintKind::Fixed(((age * 32) << 16) | ((255 - age * 8) << 8) | (age * 4))
        }
        "redstone_wire" => {
            let p: f32 = s.prop("power").and_then(|a| a.parse().ok()).unwrap_or(0.0) / 15.0;
            let r = p * 0.6 + if p > 0.0 { 0.4 } else { 0.3 };
            let g = (p * p * 0.7 - 0.5).clamp(0.0, 1.0);
            let b = (p * p * 0.6 - 0.7).clamp(0.0, 1.0);
            TintKind::Fixed(
                ((r * 255.0) as u32) << 16 | ((g * 255.0) as u32) << 8 | (b * 255.0) as u32,
            )
        }
        "water" | "water_cauldron" | "bubble_column" => TintKind::Water,
        _ => TintKind::None,
    }
}

/// Block light emission (0-15) for the common vanilla emitters.
pub fn light_emission(s: &BlockState) -> u8 {
    let n = name(s);
    let lit = s.prop("lit") == Some("true");
    match n {
        "glowstone"
        | "sea_lantern"
        | "shroomlight"
        | "jack_o_lantern"
        | "lava"
        | "fire"
        | "beacon"
        | "conduit"
        | "ochre_froglight"
        | "verdant_froglight"
        | "pearlescent_froglight"
        | "lantern"
        | "end_gateway"
        | "end_portal" => 15,
        "torch" | "wall_torch" | "end_rod" => 14,
        "campfire" if lit => 15,
        "soul_campfire" if lit => 10,
        "redstone_lamp" if lit => 15,
        "furnace" | "smoker" | "blast_furnace" if lit => 13,
        "copper_bulb"
        | "exposed_copper_bulb"
        | "waxed_copper_bulb"
        | "waxed_exposed_copper_bulb"
            if lit =>
        {
            15
        }
        "weathered_copper_bulb" | "waxed_weathered_copper_bulb" if lit => 12,
        "oxidized_copper_bulb" | "waxed_oxidized_copper_bulb" if lit => 8,
        "soul_torch" | "soul_wall_torch" | "soul_lantern" | "soul_fire" | "crying_obsidian" => 10,
        "nether_portal" => 11,
        "redstone_torch" | "redstone_wall_torch" if lit => 7,
        "redstone_ore" | "deepslate_redstone_ore" if lit => 9,
        "glow_lichen" => 7,
        "enchanting_table" | "ender_chest" => 7,
        "magma_block" => 3,
        "amethyst_cluster" => 5,
        "large_amethyst_bud" => 4,
        "medium_amethyst_bud" => 2,
        "small_amethyst_bud" | "brewing_stand" | "sculk_sensor" | "calibrated_sculk_sensor" => 1,
        "cave_vines" | "cave_vines_plant" if s.prop("berries") == Some("true") => 14,
        "sea_pickle" if s.prop("waterlogged") == Some("true") => {
            let p: u8 = s.prop("pickles").and_then(|p| p.parse().ok()).unwrap_or(1);
            3 + 3 * p
        }
        _ if n.ends_with("candle") && lit => {
            let c: u8 = s.prop("candles").and_then(|p| p.parse().ok()).unwrap_or(1);
            3 * c
        }
        _ => 0,
    }
}

/// Chunky's material emittance of a block state (0 for one that emits nothing): every
/// face of the block emits its texel colour squared times this times the emitter
/// intensity in HQ mode. Ported from Chunky (GPL-3.0, `BlockPalette.
/// getDefaultMaterialProperties` at chunky-dev/chunky 156e2bb), entry for entry, with
/// its keys as Chunky has them: `minecraft:magma` names no block since 1.13, so a
/// magma block emits nothing, as in Chunky. Candles and the `light` block are not
/// emitters here: Chunky draws a candle's flames as particles that Lumenwright does not
/// draw, and a `light` block has no faces.
pub fn emittance(s: &BlockState) -> f32 {
    let n = name(s);
    let lit = s.prop("lit") == Some("true");
    let n = n.strip_prefix("waxed_").unwrap_or(n);
    match n {
        "lava"
        | "torch"
        | "wall_torch"
        | "fire"
        | "glowstone"
        | "jack_o_lantern"
        | "beacon"
        | "sea_lantern"
        | "end_rod"
        | "lantern"
        | "shroomlight"
        | "copper_lantern"
        | "exposed_copper_lantern"
        | "weathered_copper_lantern"
        | "oxidized_copper_lantern"
        | "ochre_froglight"
        | "verdant_froglight"
        | "pearlescent_froglight"
        | "copper_torch"
        | "copper_wall_torch" => 1.0,
        "redstone_torch"
        | "redstone_wall_torch"
        | "redstone_lamp"
        | "campfire"
        | "furnace"
        | "smoker"
        | "blast_furnace"
        | "copper_bulb"
            if lit =>
        {
            1.0
        }
        "exposed_copper_bulb" if lit => 12.0 / 15.0,
        "weathered_copper_bulb" if lit => 8.0 / 15.0,
        "oxidized_copper_bulb" if lit => 4.0 / 15.0,
        "nether_portal" => 0.4,
        "soul_lantern" | "soul_torch" | "soul_wall_torch" | "soul_fire" | "crying_obsidian" => 0.6,
        "enchanting_table" => 0.5,
        "sea_pickle" if s.prop("waterlogged") == Some("true") => {
            let p: f32 = s
                .prop("pickles")
                .and_then(|p| p.parse().ok())
                .unwrap_or(1.0);
            (3.0 * p + 1.0) / 15.0
        }
        "respawn_anchor" => {
            let c: f32 = s
                .prop("charges")
                .and_then(|p| p.parse().ok())
                .unwrap_or(0.0);
            if c > 0.0 { (c * 4.0 - 2.0) / 15.0 } else { 0.0 }
        }
        "small_amethyst_bud" => 1.0 / 15.0,
        "medium_amethyst_bud" => 2.0 / 15.0,
        "large_amethyst_bud" => 4.0 / 15.0,
        "amethyst_cluster" => 5.0 / 15.0,
        "sculk_sensor" | "calibrated_sculk_sensor"
            if s.prop("sculk_sensor_phase") == Some("active") =>
        {
            1.0 / 15.0
        }
        "glow_lichen" => 7.0 / 15.0,
        "cave_vines" | "cave_vines_plant" if s.prop("berries") == Some("true") => 14.0 / 15.0,
        "sculk_catalyst" => 6.0 / 15.0,
        _ => 0.0,
    }
}

/// Whether a texture of an emissive block actually glows. Blocks like the
/// jack o'lantern or a lit furnace only emit from their lit face.
pub fn emissive_texture(s: &BlockState, texture: &str) -> bool {
    let tex = texture.rsplit('/').next().unwrap_or(texture);
    match s.short_name() {
        "jack_o_lantern" => tex == "jack_o_lantern",
        "furnace" | "smoker" | "blast_furnace" => tex.ends_with("_front_on"),
        "campfire" | "soul_campfire" => tex.contains("fire") && !tex.contains("log"),
        "respawn_anchor" => tex.contains("portal") || tex.contains("top"),
        "enchanting_table" | "ender_chest" | "brewing_stand" => false,
        _ => true,
    }
}

/// The block is water or a bubble column.
pub fn is_water(s: &BlockState) -> bool {
    matches!(name(s), "water" | "bubble_column")
}

/// The block is lava.
pub fn is_lava(s: &BlockState) -> bool {
    name(s) == "lava"
}

/// Blocks that are permanently, or currently, filled with water.
pub fn is_waterlogged(s: &BlockState) -> bool {
    s.prop("waterlogged") == Some("true")
        || matches!(
            name(s),
            "seagrass" | "tall_seagrass" | "kelp" | "kelp_plant" | "bubble_column"
        )
}

/// Blocks that skip rendering faces shared with the same block (vanilla
/// `HalfTransparentBlock.skipRendering`).
pub fn culls_same_block(s: &BlockState) -> bool {
    let n = name(s);
    matches!(
        n,
        "glass"
            | "tinted_glass"
            | "ice"
            | "frosted_ice"
            | "slime_block"
            | "honey_block"
            | "powder_snow"
    ) || n.ends_with("_stained_glass")
}

/// Fluid surface height (0..1) for a water or lava `level` property.
pub fn fluid_height(level: u32, covered: bool) -> f32 {
    if covered || level >= 8 {
        1.0
    } else {
        // Source blocks are 8/9 tall; flowing levels drop by 1/9 each.
        (8 - level.min(7)) as f32 / 9.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitters() {
        assert_eq!(light_emission(&BlockState::new("minecraft:glowstone")), 15);
        assert_eq!(
            light_emission(&BlockState::new("minecraft:furnace").with("lit", "false")),
            0
        );
        assert_eq!(
            light_emission(&BlockState::new("minecraft:furnace").with("lit", "true")),
            13
        );
        assert_eq!(
            light_emission(
                &BlockState::new("minecraft:white_candle")
                    .with("lit", "true")
                    .with("candles", "3")
            ),
            9
        );
    }

    #[test]
    fn partial_emitters() {
        let jack = BlockState::new("minecraft:jack_o_lantern");
        assert!(emissive_texture(&jack, "minecraft:block/jack_o_lantern"));
        assert!(!emissive_texture(&jack, "minecraft:block/pumpkin_side"));
        assert!(emissive_texture(
            &BlockState::new("minecraft:glowstone"),
            "minecraft:block/glowstone"
        ));
    }

    #[test]
    fn tints() {
        assert_eq!(
            tint_kind(&BlockState::new("minecraft:grass_block")),
            TintKind::Grass
        );
        assert_eq!(
            tint_kind(&BlockState::new("minecraft:birch_leaves")),
            TintKind::Fixed(0x80A755)
        );
        assert_eq!(
            tint_kind(&BlockState::new("minecraft:stone")),
            TintKind::None
        );
    }

    #[test]
    fn fluid_heights() {
        assert!((fluid_height(0, false) - 8.0 / 9.0).abs() < 1e-6);
        assert_eq!(fluid_height(0, true), 1.0);
        assert_eq!(fluid_height(8, false), 1.0);
    }
}
