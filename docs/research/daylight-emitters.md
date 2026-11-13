# Emitters in a clear daytime frame

A clear daytime HQ frame of a town drew every lantern and torch as a saturated bright
point, so the frame read as dotted with lights in broad day; more samples converge to the
same points. This note records what Chunky, the reference HQ mode is ported from, sets for
a scene's emitters, how Lumenwright's day look departed from it, and the preset it now
takes. Each statement is marked **[cited]** (Chunky's source or documentation says it),
**[measured]** or **[authored]** (Lumenwright's own rule). Chunky is read at revision
`156e2bba2526`, the source of Chunky 2.5.0-SNAPSHOT.474 (GPL-3.0, the licence its ports
here carry; see `ACKNOWLEDGEMENTS.md`).

## What Chunky sets for a new scene

| setting | Chunky's default | read at |
|---|---|---|
| emitters | **off** (`DEFAULT_EMITTERS_ENABLED = false`) | `chunky/renderer/scene/Scene.java`, the constant and the field `emittersEnabled` |
| emitter intensity | 13 (`DEFAULT_EMITTER_INTENSITY`), applied only while emitters are on | `Scene.java` |
| emitter sampling | `NONE` | `Scene.java`, the field `emitterSamplingStrategy` |
| sun | intensity 1.25 (`Sun.DEFAULT_INTENSITY`), altitude π/3, azimuth π/2.5 | `chunky/renderer/scene/sky/Sun.java` |
| sky light | 1 (`Sky.DEFAULT_INTENSITY`, for the sky's exposure and its light modifiers) | `chunky/renderer/scene/sky/Sky.java` |
| exposure | 1 (`DEFAULT_EXPOSURE`) | `Scene.java` |
| post-processing | gamma correction, gamma 2.2 (`DEFAULT_POSTPROCESSING_FILTER`, `DEFAULT_GAMMA`) | `Scene.java` |

[cited, all rows] So Chunky's default scene is a daylight scene, the sun 60° high, with its
emitters off. In `PathTracer.doDiffuseReflection` a surface adds emitted light, and emitter
sampling runs, only under `scene.emittersEnabled`; the emitted light is the texel colour
squared times the material's emittance times `emitterIntensity` [cited]. Chunky's lighting
reference (chunky-dev.github.io, *Render Controls – Lighting*) says emitters, when enabled,
"contribute lighting to the scene" and that the intensity "is a multiplier of the base
emittance value of each material"; it gives no default and no advice on day or night use
[cited]. A Chunky scene with emitters on is the scene's own choice: the benchmark scene of
[benchmark.md](../benchmark.md) states `"emittersEnabled": true` [cited,
`docs/bench/treehouse-camp-bench.json`].

## How Lumenwright's day look departed

HQ mode's day look, Chunky's look on the client's day plateau (ticks 730 to 11270, where the
client's `visual/sky_light_factor` holds its day value), took Chunky's sun 1.25, sky 1,
exposure 1 and gamma tone map, all Chunky's defaults, and drew emitters at intensity 13
with sampling `ONE` [cited, `request.rs` and `sky.rs` at `c3a8500`]. The emitters were the
one setting of the day look that was not Chunky's default. A lantern texel of linear colour
`c` emitted `13 c²` against a sunlit face's `1.25^2.2 c cos θ ≈ 1.63 c cos θ`, so a bright
texel was some eight times a sunlit face of its colour; at a distance a lantern smaller than
a pixel still carried its pixel [authored arithmetic, from the cited values].

The low-light presets were tuned with emitters on: the game look's intensity 137.9628 was
fitted at dusk ([game-sky.md](game-sky.md), *Lanterns*), and a rain or thunder frame is a
stated sky at no tick, Chunky's look at 13 under a sky dimmed to the weather's light. The
clear daytime presets were never tuned for emitters [cited, the record of the fit].

## The preset

[authored] HQ mode's default emitter intensity (`request::default_emitter_intensity`) is
0, `DAY_EMITTER_INTENSITY`, Chunky's emitters off, when the frame stands at a tick on the
client's day plateau: a vanilla sky, or a stated sky at a tick, whose
`visual/sky_light_factor` is at its day value. Everywhere else it is what it was, bit for
bit: each look's own intensity mixed by the day weight, and 13 for a stated sky at no tick.
A stated `--emitter-intensity` (`HqOptions::emitter_intensity`) is used as given. No
algorithm changed; the sun, the sky, the exposure and the tone map are untouched.

With emitters off a lantern in full daylight is drawn as its texture lit by the sun and the
sky, as a block beside it is; that is also what the game draws there, where the lightmap
clamps block light and sky light together and a sunlit face is already at full brightness
[cited: the 1.21.11 client's lightmap; recorded here only as agreement, not as the rule].

### What the preset gives up

* **An interior on the day plateau is lit by the sky alone.** A lantern in a closed room at
  noon lights nothing, as in Chunky's default scene; the game lights such a room by block
  light at any hour [authored consequence]. A frame that needs it states
  `--emitter-intensity`.
* **The default intensity steps at the day plateau's edges.** Off the plateau the intensity
  is the low-light presets' own, unchanged, so at ticks 730 and 11270 it steps between 0 and
  the mixed value (about 13 there). The sky has no step; the emitters do. One tick across
  11270 moves the showcase frame (CPU, 8 spp, 96×54) by 4.043/255 with default emitters,
  and by 0.057/255 with the emitters held at 13; one tick inside Chunky's look moves it by
  0.040/255 [measured, the test `the_default_sky_follows_the_day_without_a_jump`]. A
  continuous preset, `weight × 0 + (1 − weight) × 137.9628`, has no step but changes every
  frame off the plateau: at tick 12000 (day weight 0.6096) the intensity would fall from
  61.78 to 53.86, 12.8% lower, and at ticks 13000 and 23000 (weight 0.0749) by 0.76%
  [authored arithmetic from the timeline's 1870-tick ramps].

## Measurements

The daylight-emitters fixture (`demo-world --daylight-emitters`): a lit and an unlit
furnace, whose tops are one texture, a lantern, glowstone beside sandstone and a torch on
smooth stone in the open.

* At noon (`--time 6000`, CPU, 64 spp, 192×108), the largest channel gap between the 7×7
  means of the two furnace tops: 70.14/255 at intensity 13 (the old default; the lit top
  (236.7, 239.1, 242.6) against (166.6, 176.9, 189.4)), 2.57/255 by default now [measured,
  the test `a_daylight_emitter_is_drawn_as_its_sunlit_neighbour`, tolerance 3/255].
* At midnight the default frame is the frame at 137.9628 byte for byte, and the floor by the
  lit furnace (237.7, 229.6, 215.3) is far above the floor by the unlit one (89.1, 85.4,
  101.0), on the revision before the change and after it [measured, the test
  `a_night_emitter_still_lights_its_surroundings`].

The comparison frames are in `docs/renders/`: `daylight-emitters-noon-before.png` (emitter
intensity 13) and `daylight-emitters-noon.png` (the default), and
`daylight-emitters-midnight.png`, each drawn from the fixture by

```sh
target/release/lumenwright demo-world --daylight-emitters yard
target/release/lumenwright render -w yard --mode hq --backend cpu --spp 256 --width 384 --height 216 \
    --camera 0.5,72,7.5 --look-at 0.5,65,-0.5 --fov 55 --time 6000 -o daylight-emitters-noon.png
```

with `--emitter-intensity 13` for the frame before, and `--time 18000` for midnight.

### Eighteen stated skies, before and after

Delvewright's engine states a frame's sky from a time and a weather: six ticks (1000, 6000,
12000, 13000, 18000, 23000) under a clear sky as a stated sky at that tick by the time of
day, and under rain or thunder as a stated sky at no tick (Chunky's look, its sun dark, its
sky scaled to the weather's light). Each of the 18 was drawn through that engine's frame
path (Metal, Apple M2 Pro) with Lumenwright 0.4.0 and with this change, on two scenes: the
daylight-emitters yard (384×216, 256 spp) and a lantern-lit town seen from the
sea (640×360, 256 spp) [measured; SHA-256 of the RGBA bytes]:

* The 16 skies off the clear day plateau (clear at 12000, 13000, 18000 and 23000; rain and
  thunder at all six ticks) are byte-identical before and after, on both scenes: 32 of 32
  frames. A second run before the change reproduced every one of the 36 frames' bytes.
* The 2 clear skies on the plateau (1000, 6000) differ, on both scenes: the lanterns,
  torches and glowstone are drawn as their textures in the sun, and the lit furnace's top as
  the unlit one's.
* Rain and thunder at ticks 1000 and 6000 keep emitters at 13 under a daytime sky dimmed to
  the weather's light; there a lantern is still a bright point and the lit furnace's top
  clips to white. They are left as they were tuned.
