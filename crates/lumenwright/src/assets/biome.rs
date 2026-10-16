//! Biome climate table and colour computation (grass, foliage, water, sky).
//! Vanilla defines these in datapack JSON on the server side; the client jar
//! does not ship them, so the vanilla values are tabled here.

use super::source::ResourceStore;

#[derive(Clone, Copy, Debug)]
pub struct BiomeInfo {
    pub temperature: f32,
    pub downfall: f32,
    pub water: u32,
    pub grass: Option<u32>,
    pub foliage: Option<u32>,
    pub dark_forest: bool,
}

const WATER: u32 = 0x3F76E4;

const fn b(t: f32, d: f32) -> BiomeInfo {
    BiomeInfo {
        temperature: t,
        downfall: d,
        water: WATER,
        grass: None,
        foliage: None,
        dark_forest: false,
    }
}
const fn bw(t: f32, d: f32, water: u32) -> BiomeInfo {
    BiomeInfo {
        temperature: t,
        downfall: d,
        water,
        grass: None,
        foliage: None,
        dark_forest: false,
    }
}

pub fn biome_info(name: &str) -> BiomeInfo {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    match n {
        "plains" | "sunflower_plains" | "beach" => b(0.8, 0.4),
        "snowy_plains" | "ice_spikes" => b(0.0, 0.5),
        "desert" => b(2.0, 0.0),
        "swamp" => BiomeInfo {
            grass: Some(0x6A7039),
            foliage: Some(0x6A7039),
            ..bw(0.8, 0.9, 0x617B64)
        },
        "mangrove_swamp" => BiomeInfo {
            grass: Some(0x6A7039),
            foliage: Some(0x8DB127),
            ..bw(0.8, 0.9, 0x3A7A6A)
        },
        "forest" | "flower_forest" => b(0.7, 0.8),
        "birch_forest" | "old_growth_birch_forest" => b(0.6, 0.6),
        "dark_forest" => BiomeInfo {
            dark_forest: true,
            ..b(0.7, 0.8)
        },
        "pale_garden" => BiomeInfo {
            grass: Some(0x778272),
            foliage: Some(0x878D76),
            ..bw(0.7, 0.8, 0x76889D)
        },
        "old_growth_pine_taiga" => b(0.3, 0.8),
        "old_growth_spruce_taiga" | "taiga" => b(0.25, 0.8),
        "snowy_taiga" => bw(-0.5, 0.4, 0x3D57D6),
        "savanna" | "savanna_plateau" | "windswept_savanna" => b(2.0, 0.0),
        "windswept_hills" | "windswept_gravelly_hills" | "windswept_forest" | "stony_shore" => {
            b(0.2, 0.3)
        }
        "jungle" | "bamboo_jungle" => b(0.95, 0.9),
        "sparse_jungle" => b(0.95, 0.8),
        "badlands" | "eroded_badlands" | "wooded_badlands" => BiomeInfo {
            grass: Some(0x90814D),
            foliage: Some(0x9E814D),
            ..b(2.0, 0.0)
        },
        "meadow" => bw(0.5, 0.8, 0x0E4ECF),
        "cherry_grove" => BiomeInfo {
            grass: Some(0xB6DB61),
            foliage: Some(0xB6DB61),
            ..bw(0.5, 0.8, 0x5DB7EF)
        },
        "grove" => b(-0.2, 0.8),
        "snowy_slopes" => b(-0.3, 0.9),
        "frozen_peaks" | "jagged_peaks" => b(-0.7, 0.9),
        "stony_peaks" => b(1.0, 0.3),
        "river" | "ocean" | "deep_ocean" | "lush_caves" => b(0.5, 0.5),
        "frozen_river" => bw(0.0, 0.5, 0x3938C9),
        "snowy_beach" => bw(0.05, 0.3, 0x3D57D6),
        "warm_ocean" => bw(0.5, 0.5, 0x43D5EE),
        "lukewarm_ocean" | "deep_lukewarm_ocean" => bw(0.5, 0.5, 0x45ADF2),
        "cold_ocean" | "deep_cold_ocean" => bw(0.5, 0.5, 0x3D57D6),
        "frozen_ocean" | "deep_frozen_ocean" => bw(0.0, 0.5, 0x3938C9),
        "mushroom_fields" => b(0.9, 1.0),
        "dripstone_caves" | "deep_dark" => b(0.8, 0.4),
        "nether_wastes" | "soul_sand_valley" | "crimson_forest" | "warped_forest"
        | "basalt_deltas" => b(2.0, 0.0),
        _ => b(0.8, 0.4),
    }
}

/// Samples a 256×256 climate colormap (grass.png / foliage.png).
pub struct Colormap {
    px: Vec<u32>,
    fallback: u32,
}

impl Colormap {
    pub fn load(store: &ResourceStore, path: &str, fallback: u32) -> Colormap {
        let px = store
            .get(&format!("assets/minecraft/textures/colormap/{path}.png"))
            .and_then(|b| image::load_from_memory(b).ok())
            .map(|i| i.to_rgba8())
            .filter(|i| i.dimensions() == (256, 256))
            .map(|i| {
                i.pixels()
                    .map(|p| ((p.0[0] as u32) << 16) | ((p.0[1] as u32) << 8) | p.0[2] as u32)
                    .collect()
            })
            .unwrap_or_default();
        Colormap { px, fallback }
    }

    /// Returns 0xRRGGBB for the climate (vanilla `GrassColor.get`).
    pub fn sample(&self, temperature: f32, downfall: f32) -> u32 {
        if self.px.is_empty() {
            return self.fallback;
        }
        let t = temperature.clamp(0.0, 1.0);
        let d = downfall.clamp(0.0, 1.0) * t;
        let x = ((1.0 - t) * 255.0) as usize;
        let y = ((1.0 - d) * 255.0) as usize;
        self.px[y * 256 + x]
    }
}

pub struct BiomeColors {
    grass: Colormap,
    foliage: Colormap,
    dry_foliage: Colormap,
}

impl BiomeColors {
    pub fn new(store: &ResourceStore) -> BiomeColors {
        BiomeColors {
            grass: Colormap::load(store, "grass", 0x91BD59),
            foliage: Colormap::load(store, "foliage", 0x77AB2F),
            dry_foliage: Colormap::load(store, "dry_foliage", 0xA0773F),
        }
    }

    pub fn grass(&self, biome: &str) -> u32 {
        let i = biome_info(biome);
        if let Some(g) = i.grass {
            return g;
        }
        let c = self.grass.sample(i.temperature, i.downfall);
        if i.dark_forest {
            ((c & 0xFEFEFE) + 0x28340A) >> 1
        } else {
            c
        }
    }

    pub fn foliage(&self, biome: &str) -> u32 {
        let i = biome_info(biome);
        i.foliage
            .unwrap_or_else(|| self.foliage.sample(i.temperature, i.downfall))
    }

    pub fn dry_foliage(&self, biome: &str) -> u32 {
        let i = biome_info(biome);
        self.dry_foliage.sample(i.temperature, i.downfall)
    }

    pub fn water(&self, biome: &str) -> u32 {
        biome_info(biome).water
    }

    /// The grass colormap at a climate (an item's `minecraft:grass` tint).
    pub fn grass_climate(&self, temperature: f32, downfall: f32) -> u32 {
        self.grass.sample(temperature, downfall)
    }
}

/// Vanilla overworld sky colour for a biome temperature (`Biome.calculateSkyColor`).
pub fn sky_color(temperature: f32) -> [f32; 3] {
    let t = (temperature / 3.0).clamp(-1.0, 1.0);
    hsv_to_rgb(0.622_222_2 - t * 0.05, 0.5 + t * 0.1, 1.0)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor() as i32;
    let f = h - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plains_sky_is_vanilla_blue() {
        // Vanilla plains sky is #78A7FF.
        let c = sky_color(0.8);
        let rgb = [
            (c[0] * 255.0) as u32,
            (c[1] * 255.0) as u32,
            (c[2] * 255.0) as u32,
        ];
        assert!(
            (rgb[0] as i32 - 0x78).abs() <= 2
                && (rgb[1] as i32 - 0xA7).abs() <= 2
                && rgb[2] == 0xFF,
            "{rgb:x?}"
        );
    }

    #[test]
    fn unknown_biome_defaults() {
        let i = biome_info("dwe:lab");
        assert_eq!(i.water, WATER);
    }
}
