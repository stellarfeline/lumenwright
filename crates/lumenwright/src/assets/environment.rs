//! The sky inputs the 1.21.11 client derives from its environment attributes.
//!
//! Since 1.21.11 the sky, fog and celestial bodies are data: a dimension type
//! sets base attribute values (`attributes`), the biome at the camera overrides
//! or modifies them, and the dimension's timelines (`data/<ns>/timeline/*.json`,
//! through the `timelines` tag) apply keyframed modifiers as a function of the
//! day time. This module reads that data from the client jar and evaluates it
//! the way `net.minecraft.world.attribute.EnvironmentAttributeSystem` layers it
//! (dimension, then biome, then each timeline in tag order) and the way
//! `net.minecraft.util.KeyframeTrackSampler` samples a track (periodic segments,
//! the track's easing, then the attribute type's keyframe lerp: `ARGB.srgbLerp`
//! for colours, `Mth.lerp` for floats and angles, a step for moon phases).
//! Weather layers are not applied: the sky is drawn clear.

use std::collections::HashMap;

use serde_json::Value;

use super::source::{ResourceStore, norm_loc, split_loc};

/// Which sky a dimension draws (`DimensionType.Skybox`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skybox {
    /// No sky: only the fog colour (the Nether).
    None,
    /// Sky disc, sunrise glow, sun, moon and stars.
    Overworld,
    /// The End's textured box (drawn as its fog colour here).
    End,
}

/// The environment attribute values the sky is drawn from, at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentState {
    /// `visual/sky_color`, `0xAARRGGBB`.
    pub sky_color: u32,
    /// `visual/fog_color`, `0xAARRGGBB`.
    pub fog_color: u32,
    /// `visual/sunrise_sunset_color`, `0xAARRGGBB` (alpha is the glow's strength).
    pub sunrise_sunset_color: u32,
    /// `visual/sun_angle`, degrees (0 = noon).
    pub sun_angle: f32,
    /// `visual/moon_angle`, degrees.
    pub moon_angle: f32,
    /// `visual/star_angle`, degrees.
    pub star_angle: f32,
    /// `visual/star_brightness`, 0..1.
    pub star_brightness: f32,
    /// `visual/moon_phase`, such as `full_moon`.
    pub moon_phase: String,
    /// `visual/fog_start_distance`, blocks.
    pub fog_start: f32,
    /// `visual/fog_end_distance`, blocks.
    pub fog_end: f32,
    /// `visual/sky_fog_end_distance`, blocks.
    pub sky_fog_end: f32,
    /// `visual/sky_light_factor`, 0..1: how much of the sky's light reaches a cell open
    /// to it (the lightmap's `SkyFactor`).
    pub sky_light_factor: f32,
    /// `visual/sky_light_color`, `0xAARRGGBB`: the colour of that light (the
    /// lightmap's `SkyLightColor`).
    pub sky_light_color: u32,
    /// The dimension's skybox.
    pub skybox: Skybox,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Rgb,
    Argb,
    Float,
    Phase,
}

/// The attributes read, their type and their registered default
/// (`EnvironmentAttributes`).
const ATTRIBUTES: &[(&str, Kind, f64)] = &[
    ("visual/sky_color", Kind::Rgb, 0.0),
    ("visual/fog_color", Kind::Rgb, 0.0),
    ("visual/sunrise_sunset_color", Kind::Argb, 0.0),
    ("visual/sun_angle", Kind::Float, 0.0),
    ("visual/moon_angle", Kind::Float, 0.0),
    ("visual/star_angle", Kind::Float, 0.0),
    ("visual/star_brightness", Kind::Float, 0.0),
    ("visual/moon_phase", Kind::Phase, 0.0),
    ("visual/fog_start_distance", Kind::Float, 0.0),
    ("visual/fog_end_distance", Kind::Float, 1024.0),
    ("visual/sky_fog_end_distance", Kind::Float, 512.0),
    ("visual/sky_light_factor", Kind::Float, 1.0),
    // `-1`: opaque white, as the unsigned word `0xFFFFFFFF`.
    ("visual/sky_light_color", Kind::Rgb, 4_294_967_295.0),
];

/// The moon phases in `MoonPhase` order.
pub const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Color(u32),
    Float(f32),
    Phase(String),
}

impl Val {
    fn color(&self) -> u32 {
        match self {
            Val::Color(c) => *c,
            _ => 0,
        }
    }
    fn float(&self) -> f32 {
        match self {
            Val::Float(f) => *f,
            _ => 0.0,
        }
    }
}

/// `ExtraCodecs.STRING_RGB_COLOR` / `STRING_ARGB_COLOR`: `#rrggbb` (made opaque)
/// or `#aarrggbb`, or a packed int.
fn parse_color(v: &Value, kind: Kind) -> Option<u32> {
    if let Some(s) = v.as_str() {
        let hex = s.strip_prefix('#')?;
        let n = u32::from_str_radix(hex, 16).ok()?;
        return match (hex.len(), kind) {
            (6, Kind::Rgb) => Some(n | 0xFF00_0000),
            (8, Kind::Argb) => Some(n),
            _ => None,
        };
    }
    v.as_i64().map(|i| i as i32 as u32)
}

fn parse(v: &Value, kind: Kind) -> Option<Val> {
    match kind {
        Kind::Rgb | Kind::Argb => parse_color(v, kind).map(Val::Color),
        Kind::Float => v.as_f64().map(|f| Val::Float(f as f32)),
        Kind::Phase => v.as_str().map(|s| Val::Phase(s.to_string())),
    }
}

/// `ARGB.multiply`.
pub fn argb_multiply(a: u32, b: u32) -> u32 {
    if a == u32::MAX {
        return b;
    }
    if b == u32::MAX {
        return a;
    }
    let ch = |c: u32, s: u32| (c >> s) & 0xFF;
    [24, 16, 8, 0]
        .iter()
        .map(|&s| (ch(a, s) * ch(b, s) / 255) << s)
        .fold(0, |acc, v| acc | v)
}

/// `Mth.lerpInt`.
fn lerp_int(t: f32, a: i32, b: i32) -> i32 {
    a + (t * (b - a) as f32).floor() as i32
}

/// `ARGB.srgbLerp`: each channel, alpha included, lerped as stored.
pub fn srgb_lerp(t: f32, a: u32, b: u32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xFF) as i32;
    [24, 16, 8, 0]
        .iter()
        .map(|&s| (lerp_int(t, ch(a, s), ch(b, s)) as u32 & 0xFF) << s)
        .fold(0, |acc, v| acc | v)
}

/// Applies a modifier (`AttributeModifier.OperationId`) to `base`. Only the
/// operations the vanilla data uses on these attributes are implemented; any
/// other is logged and treated as `override`.
fn apply(modifier: &str, base: &Val, arg: &Val) -> Val {
    match (modifier, base, arg) {
        ("override", _, a) => a.clone(),
        ("multiply", Val::Color(b), Val::Color(a)) => Val::Color(argb_multiply(*b, *a)),
        ("multiply", Val::Float(b), Val::Float(a)) => Val::Float(b * a),
        ("add", Val::Float(b), Val::Float(a)) => Val::Float(b + a),
        ("subtract", Val::Float(b), Val::Float(a)) => Val::Float(b - a),
        ("maximum", Val::Float(b), Val::Float(a)) => Val::Float(b.max(*a)),
        ("minimum", Val::Float(b), Val::Float(a)) => Val::Float(b.min(*a)),
        (m, _, a) => {
            log::warn!("environment attribute modifier {m:?} is not implemented; overriding");
            a.clone()
        }
    }
}

/// `EasingType`: `linear`, `constant`, a cubic Bézier (`{"cubic_bezier": [x1, y1, x2, y2]}`),
/// or one of the named curves the sky data does not use (treated as linear, logged).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ease {
    Linear,
    Constant,
    Bezier([f32; 4]),
}

impl Ease {
    fn parse(v: &Value) -> Ease {
        if let Some(s) = v.as_str() {
            return match s {
                "linear" => Ease::Linear,
                "constant" => Ease::Constant,
                other => {
                    log::warn!("easing {other:?} is not implemented; linear");
                    Ease::Linear
                }
            };
        }
        if let Some(a) = v["cubic_bezier"].as_array()
            && a.len() == 4
        {
            let f = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            return Ease::Bezier([f(0), f(1), f(2), f(3)]);
        }
        Ease::Linear
    }

    /// `EasingType.apply`; the Bézier is solved for x with four Newton steps.
    fn apply(self, t: f32) -> f32 {
        match self {
            Ease::Linear => t,
            Ease::Constant => 0.0,
            Ease::Bezier([x1, y1, x2, y2]) => {
                let curve =
                    |p1: f32, p2: f32| (3.0 * p1 - 3.0 * p2 + 1.0, -6.0 * p1 + 3.0 * p2, 3.0 * p1);
                let (xa, xb, xc) = curve(x1, x2);
                let (ya, yb, yc) = curve(y1, y2);
                let mut s = t;
                for _ in 0..4 {
                    let grad = (3.0 * xa * s + 2.0 * xb) * s + xc;
                    if grad < 1.0e-5 {
                        break;
                    }
                    let x = ((xa * s + xb) * s + xc) * s - t;
                    s -= x / grad;
                }
                ((ya * s + yb) * s + yc) * s
            }
        }
    }
}

/// One baked track: segments `(from_ticks, from, to_ticks, to)` as
/// `KeyframeTrackSampler.bakeSegments` lays them out.
struct Track {
    kind: Kind,
    modifier: String,
    ease: Ease,
    segments: Vec<(i64, Val, i64, Val)>,
    period: Option<i64>,
}

impl Track {
    fn parse(v: &Value, kind: Kind, period: Option<i64>) -> Option<Track> {
        let mut keys = Vec::new();
        for k in v["keyframes"].as_array()? {
            keys.push((k["ticks"].as_i64()?, parse(&k["value"], kind)?));
        }
        if keys.is_empty() {
            return None;
        }
        let ease = Ease::parse(&v["ease"]);
        let mut segments = Vec::new();
        if keys.len() == 1 {
            let v = keys[0].1.clone();
            segments.push((0, v.clone(), 0, v));
        } else {
            let (first, last) = (keys[0].clone(), keys[keys.len() - 1].clone());
            if let Some(p) = period {
                segments.push((last.0 - p, last.1.clone(), first.0, first.1.clone()));
            }
            for w in keys.windows(2) {
                segments.push((w[0].0, w[0].1.clone(), w[1].0, w[1].1.clone()));
            }
            if let Some(p) = period {
                segments.push((last.0, last.1, first.0 + p, first.1));
            }
        }
        let single = keys.len() == 1;
        Some(Track {
            kind,
            modifier: v["modifier"].as_str().unwrap_or("override").to_string(),
            ease: if single { Ease::Constant } else { ease },
            segments,
            period,
        })
    }

    /// `KeyframeTrackSampler.sample`.
    fn sample(&self, ticks: i64) -> Val {
        let t = match self.period {
            Some(p) => ticks.rem_euclid(p),
            None => ticks,
        };
        let seg = self
            .segments
            .iter()
            .find(|s| t < s.2)
            .unwrap_or(self.segments.last().unwrap());
        if t <= seg.0 {
            return seg.1.clone();
        }
        if t >= seg.2 {
            return seg.3.clone();
        }
        let f = (t - seg.0) as f32 / (seg.2 - seg.0) as f32;
        let e = self.ease.apply(f);
        match (self.kind, &seg.1, &seg.3) {
            (Kind::Rgb | Kind::Argb, Val::Color(a), Val::Color(b)) => {
                Val::Color(srgb_lerp(e, *a, *b))
            }
            (Kind::Float, Val::Float(a), Val::Float(b)) => Val::Float(a + e * (b - a)),
            // Not interpolated: `LerpFunction.ofStep(1)` keeps the earlier value.
            (_, a, _) => a.clone(),
        }
    }
}

fn read_json(store: &ResourceStore, path: &str) -> Option<Value> {
    serde_json::from_slice(store.get(path)?).ok()
}

/// `data/<ns>/<dir>/<path>.json` for a resource location.
fn data_path(dir: &str, loc: &str) -> String {
    let (ns, path) = split_loc(loc);
    format!("data/{ns}/{dir}/{path}.json")
}

/// The timelines a `timelines` field names: a tag (`#ns:path`, nested tags
/// resolved), a single id, or a list.
fn resolve_timelines(store: &ResourceStore, v: &Value, out: &mut Vec<String>, depth: u32) {
    if depth > 8 {
        return;
    }
    match v {
        Value::String(s) => {
            if let Some(tag) = s.strip_prefix('#') {
                let Some(t) = read_json(store, &data_path("tags/timeline", tag)) else {
                    log::warn!("timeline tag #{tag} is missing");
                    return;
                };
                for e in t["values"].as_array().into_iter().flatten() {
                    let e = e
                        .as_str()
                        .map(|s| Value::String(s.to_string()))
                        .unwrap_or_else(|| e["id"].clone());
                    resolve_timelines(store, &e, out, depth + 1);
                }
            } else {
                let id = norm_loc(s);
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        }
        Value::Array(a) => a
            .iter()
            .for_each(|e| resolve_timelines(store, e, out, depth + 1)),
        _ => {}
    }
}

/// Reads the sky's environment attributes for a dimension (`minecraft:overworld`),
/// the biome at the camera and a day time in ticks (the save's `DayTime`, not
/// reduced to one day: the moon's timeline spans eight).
///
/// `None` when the jar carries no dimension type data with attributes (clients
/// before 1.21.11); the caller then has no vanilla sky to draw.
pub fn sample(
    store: &ResourceStore,
    dimension: &str,
    biome: &str,
    day_time: i64,
) -> Option<EnvironmentState> {
    let dim = read_json(store, &data_path("dimension_type", dimension))?;
    if !dim["attributes"].is_object() {
        return None;
    }
    let biome_json = read_json(store, &data_path("worldgen/biome", biome));
    let mut timelines = Vec::new();
    resolve_timelines(store, &dim["timelines"], &mut timelines, 0);
    let timelines: Vec<Value> = timelines
        .iter()
        .filter_map(|id| read_json(store, &data_path("timeline", id)))
        .collect();

    let mut values: HashMap<&str, Val> = HashMap::new();
    for &(name, kind, default) in ATTRIBUTES {
        let key = format!("minecraft:{name}");
        let mut v = match kind {
            Kind::Rgb | Kind::Argb => Val::Color(default as u32),
            Kind::Float => Val::Float(default as f32),
            Kind::Phase => Val::Phase(MOON_PHASES[0].to_string()),
        };
        // Constant layers: the dimension, then the biome (a plain value overrides;
        // `{"argument", "modifier"}` modifies).
        let layers = [
            Some(&dim["attributes"]),
            biome_json.as_ref().map(|b| &b["attributes"]),
        ];
        for attrs in layers.into_iter().flatten() {
            let a = &attrs[&key];
            if a.is_null() {
                continue;
            }
            if let Some(m) = a["modifier"].as_str() {
                if let Some(arg) = parse(&a["argument"], kind) {
                    v = apply(m, &v, &arg);
                }
            } else if let Some(arg) = parse(a, kind) {
                v = arg;
            }
        }
        for tl in &timelines {
            let period = tl["period_ticks"].as_i64();
            if let Some(track) = Track::parse(&tl["tracks"][&key], kind, period) {
                let arg = track.sample(day_time);
                v = apply(&track.modifier, &v, &arg);
            }
        }
        values.insert(name, v);
    }
    let skybox = match dim["skybox"].as_str().unwrap_or("overworld") {
        "none" => Skybox::None,
        "end" => Skybox::End,
        _ => Skybox::Overworld,
    };
    let f = |n: &str| values[n].float();
    Some(EnvironmentState {
        sky_color: values["visual/sky_color"].color(),
        fog_color: values["visual/fog_color"].color(),
        sunrise_sunset_color: values["visual/sunrise_sunset_color"].color(),
        sun_angle: f("visual/sun_angle"),
        moon_angle: f("visual/moon_angle"),
        star_angle: f("visual/star_angle"),
        star_brightness: f("visual/star_brightness"),
        moon_phase: match &values["visual/moon_phase"] {
            Val::Phase(p) => p.clone(),
            _ => MOON_PHASES[0].to_string(),
        },
        fog_start: f("visual/fog_start_distance"),
        fog_end: f("visual/fog_end_distance"),
        sky_fog_end: f("visual/sky_fog_end_distance"),
        sky_light_factor: f("visual/sky_light_factor"),
        sky_light_color: values["visual/sky_light_color"].color(),
        skybox,
    })
}

/// The night and day values of `visual/sky_light_factor` over a dimension's timelines:
/// its least and greatest value at the biome, sampled at every keyframe tick of every
/// timeline's track of it (the track is piecewise linear, so its extremes are at
/// keyframes). `None` without environment attributes or without such a track.
pub fn sky_light_factor_range(
    store: &ResourceStore,
    dimension: &str,
    biome: &str,
) -> Option<(f32, f32)> {
    let dim = read_json(store, &data_path("dimension_type", dimension))?;
    let mut timelines = Vec::new();
    resolve_timelines(store, &dim["timelines"], &mut timelines, 0);
    let mut ticks = Vec::new();
    for id in &timelines {
        let Some(tl) = read_json(store, &data_path("timeline", id)) else {
            continue;
        };
        for k in tl["tracks"]["minecraft:visual/sky_light_factor"]["keyframes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(t) = k["ticks"].as_i64() {
                ticks.push(t);
            }
        }
    }
    if ticks.is_empty() {
        return None;
    }
    let values: Vec<f32> = ticks
        .iter()
        .filter_map(|&t| sample(store, dimension, biome, t).map(|e| e.sky_light_factor))
        .collect();
    let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    (lo <= hi).then_some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ResourceStore {
        let mut s = ResourceStore::new();
        s.insert(
            "data/minecraft/dimension_type/overworld.json",
            br##"{"attributes":{"minecraft:visual/sky_color":"#78a7ff","minecraft:visual/fog_color":"#c0d8ff"},"timelines":"#minecraft:in_overworld"}"##.to_vec(),
        );
        s.insert(
            "data/minecraft/tags/timeline/in_overworld.json",
            br#"{"values":["minecraft:day"]}"#.to_vec(),
        );
        s.insert(
            "data/minecraft/timeline/day.json",
            br##"{"period_ticks":24000,"tracks":{
                "minecraft:visual/sky_color":{"keyframes":[{"ticks":133,"value":"#ffffff"},{"ticks":11867,"value":"#ffffff"},{"ticks":13670,"value":"#000000"},{"ticks":22330,"value":"#000000"}],"modifier":"multiply"},
                "minecraft:visual/sun_angle":{"ease":{"cubic_bezier":[0.362,0.241,0.638,0.759]},"keyframes":[{"ticks":6000,"value":360.0},{"ticks":6000,"value":0.0}]},
                "minecraft:visual/star_brightness":{"keyframes":[{"ticks":11373,"value":0.0},{"ticks":13228,"value":0.5},{"ticks":22772,"value":0.5},{"ticks":23758,"value":0.0}],"modifier":"maximum"},
                "minecraft:visual/sky_light_color":{"keyframes":[{"ticks":730,"value":"#ffffff"},{"ticks":11270,"value":"#ffffff"},{"ticks":13140,"value":"#7a7aff"},{"ticks":22860,"value":"#7a7aff"}],"modifier":"multiply"},
                "minecraft:visual/sky_light_factor":{"keyframes":[{"ticks":730,"value":1.0},{"ticks":11270,"value":1.0},{"ticks":13140,"value":0.24},{"ticks":22860,"value":0.24}],"modifier":"multiply"}
            }}"##
                .to_vec(),
        );
        s
    }

    #[test]
    fn day_cycle_layers_dimension_and_timeline() {
        let s = store();
        let noon = sample(&s, "minecraft:overworld", "minecraft:plains", 6000).unwrap();
        assert_eq!(noon.sky_color, 0xFF78_A7FF);
        assert_eq!(noon.fog_color, 0xFFC0_D8FF);
        assert!(noon.sun_angle.abs() < 1e-3, "{}", noon.sun_angle);
        assert_eq!(noon.star_brightness, 0.0);
        let night = sample(&s, "minecraft:overworld", "minecraft:plains", 18000).unwrap();
        assert_eq!(night.sky_color, 0xFF00_0000);
        assert!(
            (night.sun_angle - 180.0).abs() < 1e-3,
            "{}",
            night.sun_angle
        );
        assert_eq!(night.star_brightness, 0.5);
        // The lightmap's two inputs: the timeline multiplies the registered defaults,
        // opaque white and 1.
        assert_eq!(noon.sky_light_color, 0xFFFF_FFFF);
        assert_eq!(noon.sky_light_factor, 1.0);
        assert_eq!(night.sky_light_color, 0xFF7A_7AFF);
        assert!((night.sky_light_factor - 0.24).abs() < 1e-6);
        // Half way down the dusk ramp: each 8-bit channel lerped as stored.
        let dusk = sample(&s, "minecraft:overworld", "minecraft:plains", 12205).unwrap();
        assert_eq!(dusk.sky_light_color, 0xFFBC_BCFF);
        assert!((dusk.sky_light_factor - 0.62).abs() < 1e-5);
        // Its night and day values over the timeline.
        let (lo, hi) =
            sky_light_factor_range(&s, "minecraft:overworld", "minecraft:plains").unwrap();
        assert!((lo - 0.24).abs() < 1e-6 && hi == 1.0, "{lo} {hi}");
        // Day 3 at noon: the same as day 0.
        let later = sample(
            &s,
            "minecraft:overworld",
            "minecraft:plains",
            3 * 24000 + 6000,
        )
        .unwrap();
        assert_eq!(later, noon);
    }

    #[test]
    fn colour_helpers_match_argb() {
        assert_eq!(argb_multiply(u32::MAX, 0x1234_5678), 0x1234_5678);
        assert_eq!(argb_multiply(0xFF80_8080, 0xFFFF_0000), 0xFF80_0000);
        assert_eq!(srgb_lerp(0.5, 0xFF00_0000, 0xFFFF_FFFF), 0xFF7F_7F7F);
    }

    #[test]
    fn no_attributes_is_none() {
        assert!(
            sample(
                &ResourceStore::new(),
                "minecraft:overworld",
                "minecraft:plains",
                0
            )
            .is_none()
        );
    }
}
